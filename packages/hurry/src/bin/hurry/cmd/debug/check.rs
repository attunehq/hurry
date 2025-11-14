use std::{
    collections::HashMap,
    ffi::OsStr,
    hash::{Hash, Hasher},
    io::Cursor,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};

use cargo_metadata::Message;
use clap::Args;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use derive_more::Debug;
use futures::TryStreamExt as _;
use rustc_stable_hash::StableSipHasher128;
use serde::{Deserialize, Serialize, de, ser};
use tracing::{debug, info, instrument, trace, warn};
use url::Url;

use hurry::{
    cargo::{
        self, BuiltArtifact, CargoBuildArguments, CargoCache, Fingerprint, Handles, Profile,
        QualifiedPath, Workspace, build_script2, dep_info2,
        fingerprint::{self, DepFingerprint},
        path2,
        workspace2::{
            self, BuildScriptCompilationUnitPlan, BuildScriptExecutionUnitPlan,
            LibraryCrateUnitPlan,
        },
    },
    cas::FsCas,
    fs, mk_rel_dir,
    path::{AbsDirPath, AbsFilePath, JoinWith as _, TryJoinWith as _},
    progress::TransferBar,
};

#[derive(Clone, Args, Debug)]
pub struct Options {
    #[arg(long)]
    restore_only: bool,
    #[arg(long)]
    skip_restore: bool,

    /// These arguments are passed directly to `cargo build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    argv: Vec<String>,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    // Parse and validate cargo build arguments.
    let args = CargoBuildArguments::from_iter(&options.argv);
    debug!(?args, "parsed cargo build arguments");

    // Open workspace.
    let workspace = workspace2::Workspace::from_argv(&args)
        .await
        .context("opening workspace")?;

    let unit_plan = workspace.unit_plan(args).await?;
    info!(?unit_plan, "unit plan");

    // // Set up prototype CAS.
    // let cas = {
    //     let cas_path = AbsDirPath::try_from("/tmp/hurry/cas")?;
    //     fs::create_dir_all(&cas_path).await?;
    //     FsCas::open_dir(&cas_path).await?
    // };

    // Set up prototype cache. In this cache path, we save information about
    // units in structs serialized to JSON and saved under the unit hash.
    let cache_path = AbsDirPath::try_from("/tmp/hurry/cache")?;
    fs::create_dir_all(&cache_path).await?;

    // Restore artifacts.
    if !options.skip_restore {
        let mut rewritten_fingerprints: HashMap<u64, Arc<Fingerprint>> = HashMap::new();
        for unit in unit_plan.clone() {
            debug!(?unit, "restoring unit");

            let info = unit.info();
            let profile_dir = match info.target_arch {
                Some(_) => workspace.target_profile_dir(),
                None => workspace.host_profile_dir(),
            };

            // TODO: Check whether the unit is fresh on disk. If so, skip restoring
            // it.

            let saved: SavedUnit = {
                let contents = fs::must_read_buffered(
                    &cache_path.try_join_file(format!("{}.json", info.unit_hash))?,
                )
                .await?;
                serde_json::from_slice(&contents)?
            };
            match (saved, unit) {
                (
                    SavedUnit::LibraryCrate(lib_files, saved_unit),
                    workspace2::UnitPlan::LibraryCrate(lib_unit),
                ) => {
                    // TODO: Is this overly strict? For example, is it possible to
                    // have a hash mismatch if the dep indexes are different, but
                    // still have a valid restore?
                    if saved_unit.info != lib_unit.info {
                        bail!("restored unit mismatch");
                    }

                    // Restore output files.
                    for saved_file in lib_files.output_files {
                        let path = saved_file
                            .path
                            .reconstruct(&workspace, &lib_unit.info)
                            .try_as_abs_file()?;
                        fs::write(&path, saved_file.contents).await?;
                        fs::set_executable(&path, saved_file.executable).await?;
                    }

                    // Restore encoded Cargo dep-info file.
                    fs::write(
                        &profile_dir.join(&lib_unit.encoded_dep_info_file()?),
                        lib_files.encoded_dep_info_file,
                    )
                    .await?;

                    // Reconstruct and restore rustc dep-info file.
                    fs::write(
                        &profile_dir.join(&lib_unit.dep_info_file()?),
                        lib_files
                            .dep_info_file
                            .reconstruct(&workspace, &lib_unit.info),
                    )
                    .await?;

                    // Reconstruct and restore fingerprint.
                    let mut saved_fingerprint = lib_files.fingerprint;
                    let old_fingerprint_hash = saved_fingerprint.hash_u64();

                    // First, rewrite the `path` field.
                    saved_fingerprint.path =
                        fingerprint::util_hash_u64(PathBuf::from(&lib_unit.src_path));
                    debug!(path = ?PathBuf::from(&lib_unit.src_path), path_hash = ?saved_fingerprint.path, "rewritten fingerprint");

                    // Then, rewrite the `deps` field.
                    //
                    // We don't actually have enough information to synthesize our
                    // own DepFingerprints (in particular, it would be very annoying
                    // to derive `only_requires_rmeta` independently). But the old
                    // fingerprint hashes are unique, and we know our old
                    // fingerprint hash! So we save a map of the old fingerprint
                    // hashes to the replacement fingerprint hashes, and use that to
                    // look up the correct replacement fingerprint hash in future
                    // DepFingerprints, leaving all other fields untouched.
                    //
                    // This works because we know the units are in dependency order,
                    // so previous replacement fingerprint hashes will always have
                    // already been calculated when we need them.
                    debug!("rewrite fingerprint deps: start");
                    for dep in saved_fingerprint.deps.iter_mut() {
                        debug!(?dep, "rewriting fingerprint dep");
                        let old_dep_fingerprint = dep.fingerprint.hash_u64();
                        dep.fingerprint = rewritten_fingerprints
                            .get(&old_dep_fingerprint)
                            .ok_or_eyre("dependency fingerprint hash not found")?
                            .clone();
                    }
                    debug!("rewrite fingerprint deps: done");

                    // Clear and recalculate fingerprint hash.
                    saved_fingerprint.clear_memoized();
                    let fingerprint_hash = saved_fingerprint.fingerprint_hash();
                    debug!(old = ?old_fingerprint_hash, new = ?saved_fingerprint.hash_u64(), "rewritten fingerprint hash");

                    // Finally, write the reconstructed fingerprint.
                    fs::write(
                        &profile_dir.join(&lib_unit.fingerprint_hash_file()?),
                        fingerprint_hash,
                    )
                    .await?;
                    fs::write(
                        &profile_dir.join(&lib_unit.fingerprint_json_file()?),
                        serde_json::to_vec(&saved_fingerprint)?,
                    )
                    .await?;

                    // Save unit fingerprint (for future dependents).
                    rewritten_fingerprints
                        .insert(old_fingerprint_hash, Arc::new(saved_fingerprint));

                    // TODO: Set timestamps.
                }
                (
                    SavedUnit::BuildScriptCompilation(bsc_files, saved_unit),
                    workspace2::UnitPlan::BuildScriptCompilation(bsc_unit),
                ) => {
                    if saved_unit.info != bsc_unit.info {
                        bail!("restored unit mismatch");
                    }

                    // Restore compiled build script program.
                    let program_file = profile_dir.join(bsc_unit.program_file()?);
                    fs::write(&program_file, bsc_files.compiled_program).await?;
                    fs::set_executable(&program_file, true).await?;
                    fs::hard_link(
                        &program_file,
                        &profile_dir.join(bsc_unit.linked_program_file()?),
                    )
                    .await?;

                    // Restore encoded Cargo dep-info file.
                    fs::write(
                        &profile_dir.join(&bsc_unit.encoded_dep_info_file()?),
                        bsc_files.encoded_dep_info_file,
                    )
                    .await?;

                    // Reconstruct and restore rustc dep-info file.
                    fs::write(
                        &profile_dir.join(&bsc_unit.dep_info_file()?),
                        bsc_files
                            .dep_info_file
                            .reconstruct(&workspace, &bsc_unit.info),
                    )
                    .await?;

                    // Reconstruct and restore fingerprint.
                    let mut saved_fingerprint = bsc_files.fingerprint;
                    let old_fingerprint_hash = saved_fingerprint.hash_u64();

                    // First, rewrite the `path` field.
                    saved_fingerprint.path =
                        fingerprint::util_hash_u64(PathBuf::from(&bsc_unit.src_path));
                    debug!(path = ?PathBuf::from(&bsc_unit.src_path), path_hash = ?saved_fingerprint.path, "rewritten fingerprint");

                    // Then, rewrite the `deps` field.
                    //
                    // We don't actually have enough information to synthesize our
                    // own DepFingerprints (in particular, it would be very annoying
                    // to derive `only_requires_rmeta` independently). But the old
                    // fingerprint hashes are unique, and we know our old
                    // fingerprint hash! So we save a map of the old fingerprint
                    // hashes to the replacement fingerprint hashes, and use that to
                    // look up the correct replacement fingerprint hash in future
                    // DepFingerprints, leaving all other fields untouched.
                    //
                    // This works because we know the units are in dependency order,
                    // so previous replacement fingerprint hashes will always have
                    // already been calculated when we need them.
                    debug!("rewrite fingerprint deps: start");
                    for dep in saved_fingerprint.deps.iter_mut() {
                        debug!(?dep, "rewriting fingerprint dep");
                        let old_dep_fingerprint = dep.fingerprint.hash_u64();
                        dep.fingerprint = rewritten_fingerprints
                            .get(&old_dep_fingerprint)
                            .ok_or_eyre("dependency fingerprint hash not found")?
                            .clone();
                    }
                    debug!("rewrite fingerprint deps: done");

                    // Clear and recalculate fingerprint hash.
                    saved_fingerprint.clear_memoized();
                    let fingerprint_hash = saved_fingerprint.fingerprint_hash();
                    debug!(old = ?old_fingerprint_hash, new = ?saved_fingerprint.hash_u64(), "rewritten fingerprint hash");

                    // Finally, write the reconstructed fingerprint.
                    fs::write(
                        &profile_dir.join(&bsc_unit.fingerprint_hash_file()?),
                        fingerprint_hash,
                    )
                    .await?;
                    fs::write(
                        &profile_dir.join(&bsc_unit.fingerprint_json_file()?),
                        serde_json::to_vec(&saved_fingerprint)?,
                    )
                    .await?;

                    // Save unit fingerprint (for future dependents).
                    rewritten_fingerprints
                        .insert(old_fingerprint_hash, Arc::new(saved_fingerprint));
                }
                (
                    SavedUnit::BuildScriptExecution(bse_files, saved_unit),
                    workspace2::UnitPlan::BuildScriptExecution(bse_unit),
                ) => {
                    if saved_unit.info != bse_unit.info {
                        bail!("restored unit mismatch");
                    }

                    // Restore OUT_DIR files.
                    for saved_file in bse_files.out_dir_files {
                        let path = saved_file
                            .path
                            .reconstruct(&workspace, &bse_unit.info)
                            .try_as_abs_file()?;
                        fs::write(&path, saved_file.contents).await?;
                        fs::set_executable(&path, saved_file.executable).await?;
                    }

                    // Reconstruct and restore build script STDOUT.
                    fs::write(
                        &profile_dir.join(&bse_unit.stdout_file()?),
                        bse_files.stdout.reconstruct(&workspace, &bse_unit.info),
                    )
                    .await?;

                    // Restore build script STDERR.
                    fs::write(
                        &profile_dir.join(&bse_unit.stderr_file()?),
                        bse_files.stderr,
                    )
                    .await?;

                    // Generate `root-output` file.
                    fs::write(
                        &profile_dir.join(&bse_unit.root_output_file()?),
                        bse_unit.out_dir()?.as_os_str().as_encoded_bytes(),
                    )
                    .await?;

                    // Reconstruct and restore fingerprint.
                    let mut saved_fingerprint = bse_files.fingerprint;
                    let old_fingerprint_hash = saved_fingerprint.hash_u64();

                    // Rewrite the `deps` field. Note that we never need to rewrite
                    // the `path` field for build script execution units, since it's
                    // always unset[^1].
                    //
                    // We don't actually have enough information to synthesize our
                    // own DepFingerprints (in particular, it would be very annoying
                    // to derive `only_requires_rmeta` independently). But the old
                    // fingerprint hashes are unique, and we know our old
                    // fingerprint hash! So we save a map of the old fingerprint
                    // hashes to the replacement fingerprint hashes, and use that to
                    // look up the correct replacement fingerprint hash in future
                    // DepFingerprints, leaving all other fields untouched.
                    //
                    // This works because we know the units are in dependency order,
                    // so previous replacement fingerprint hashes will always have
                    // already been calculated when we need them.
                    //
                    // [^1]: https://github.com/attunehq/cargo/blob/21f1bfe23aa3fafd6205b8e3368a499466336bb9/src/cargo/core/compiler/fingerprint/mod.rs#L1696
                    debug!("rewrite fingerprint deps: start");
                    for dep in saved_fingerprint.deps.iter_mut() {
                        debug!(?dep, "rewriting fingerprint dep");
                        let old_dep_fingerprint = dep.fingerprint.hash_u64();
                        dep.fingerprint = rewritten_fingerprints
                            .get(&old_dep_fingerprint)
                            .ok_or_else(|| {
                                eyre!("dependency fingerprint hash not found").wrap_err(format!(
                                    "rewriting fingerprint {} for unit {}",
                                    old_dep_fingerprint, bse_unit.info.unit_hash
                                ))
                            })?
                            .clone();
                    }
                    debug!("rewrite fingerprint deps: done");

                    // Clear and recalculate fingerprint hash.
                    saved_fingerprint.clear_memoized();
                    let fingerprint_hash = saved_fingerprint.fingerprint_hash();
                    debug!(old = ?old_fingerprint_hash, new = ?saved_fingerprint.hash_u64(), "rewritten fingerprint hash");

                    // Finally, write the reconstructed fingerprint.
                    fs::write(
                        &profile_dir.join(&bse_unit.fingerprint_hash_file()?),
                        fingerprint_hash,
                    )
                    .await?;
                    fs::write(
                        &profile_dir.join(&bse_unit.fingerprint_json_file()?),
                        serde_json::to_vec(&saved_fingerprint)?,
                    )
                    .await?;

                    // Save unit fingerprint (for future dependents).
                    rewritten_fingerprints
                        .insert(old_fingerprint_hash, Arc::new(saved_fingerprint));
                }
                _ => bail!("restored unit mismatch"),
            }
        }
    }

    if options.restore_only {
        return Ok(());
    }

    // Run build.
    cargo::invoke("build", &options.argv)
        .await
        .context("build with cargo")?;

    // Save artifacts.
    for unit in unit_plan {
        let unit_info = unit.info();
        // Support cross-compilation. Note that some library crates may be built
        // on the host even when `--target` is set (e.g. proc macros and build
        // script dependencies). This field already correctly sets the
        // `target_arch` value taking that into account.
        let profile_dir = match &unit_info.target_arch {
            Some(_) => workspace.target_profile_dir(),
            None => workspace.host_profile_dir(),
        };

        let saved = match &unit {
            workspace2::UnitPlan::LibraryCrate(
                lib_unit @ LibraryCrateUnitPlan {
                    info: _,
                    src_path: _,
                    outputs,
                },
            ) => {
                let output_files = {
                    let mut output_files = Vec::new();
                    for output_file_path in outputs.into_iter() {
                        let path = path2::QualifiedPath::parse(
                            &workspace,
                            &unit_info,
                            output_file_path.as_std_path(),
                        )
                        .await?;
                        let contents = fs::must_read_buffered(&output_file_path).await?;
                        let executable = fs::is_executable(&output_file_path.as_std_path()).await;
                        output_files.push(SavedFile {
                            path,
                            contents,
                            executable,
                        });
                    }
                    output_files
                };

                let dep_info_file = dep_info2::DepInfo::from_file(
                    &workspace,
                    &unit_info,
                    &profile_dir.join(&lib_unit.dep_info_file()?),
                )
                .await?;

                let encoded_dep_info_file =
                    fs::must_read_buffered(&profile_dir.join(&lib_unit.encoded_dep_info_file()?))
                        .await?;

                let fingerprint = {
                    let fingerprint_json = fs::must_read_buffered_utf8(
                        &profile_dir.join(&lib_unit.fingerprint_json_file()?),
                    )
                    .await?;
                    let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

                    let fingerprint_hash = fs::must_read_buffered_utf8(
                        &profile_dir.join(&lib_unit.fingerprint_hash_file()?),
                    )
                    .await?;

                    // Sanity check that the fingerprint hashes match.
                    if fingerprint.fingerprint_hash() != fingerprint_hash {
                        bail!("fingerprint hash mismatch");
                    }

                    fingerprint
                };

                SavedUnit::LibraryCrate(
                    LibraryFiles {
                        output_files,
                        dep_info_file,
                        fingerprint,
                        encoded_dep_info_file,
                    },
                    lib_unit.clone(),
                )
            }
            workspace2::UnitPlan::BuildScriptCompilation(bsc_unit) => {
                let compiled_program =
                    fs::must_read_buffered(&profile_dir.join(bsc_unit.program_file()?)).await?;

                let dep_info_file = dep_info2::DepInfo::from_file(
                    &workspace,
                    &unit_info,
                    &profile_dir.join(&bsc_unit.dep_info_file()?),
                )
                .await?;

                let encoded_dep_info_file =
                    fs::must_read_buffered(&profile_dir.join(&bsc_unit.encoded_dep_info_file()?))
                        .await?;

                let fingerprint = {
                    let fingerprint_json = fs::must_read_buffered_utf8(
                        &profile_dir.join(&bsc_unit.fingerprint_json_file()?),
                    )
                    .await?;
                    let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

                    let fingerprint_hash = fs::must_read_buffered_utf8(
                        &profile_dir.join(&bsc_unit.fingerprint_hash_file()?),
                    )
                    .await?;

                    // Sanity check that the fingerprint hashes match.
                    if fingerprint.fingerprint_hash() != fingerprint_hash {
                        bail!("fingerprint hash mismatch");
                    }

                    fingerprint
                };

                SavedUnit::BuildScriptCompilation(
                    BuildScriptCompiledFiles {
                        compiled_program,
                        dep_info_file,
                        encoded_dep_info_file,
                        fingerprint,
                    },
                    bsc_unit.clone(),
                )
            }
            workspace2::UnitPlan::BuildScriptExecution(bse_unit) => {
                let stdout = build_script2::BuildScriptOutput::from_file(
                    &workspace,
                    &unit_info,
                    &profile_dir.join(&bse_unit.stdout_file()?),
                )
                .await?;
                let stderr =
                    fs::must_read_buffered(&profile_dir.join(&bse_unit.stderr_file()?)).await?;
                let out_dir_files = {
                    let files = fs::walk_files(&profile_dir.join(&bse_unit.out_dir()?))
                        .try_collect::<Vec<_>>()
                        .await?;
                    let mut out_dir_files = Vec::new();
                    for file in files {
                        let path =
                            path2::QualifiedPath::parse(&workspace, &unit_info, file.as_std_path())
                                .await?;
                        let executable = fs::is_executable(file.as_std_path()).await;
                        let contents = fs::must_read_buffered(&file).await?;
                        out_dir_files.push(SavedFile {
                            path,
                            executable,
                            contents,
                        });
                    }
                    out_dir_files
                };

                let fingerprint = {
                    let fingerprint_json = fs::must_read_buffered_utf8(
                        &profile_dir.join(bse_unit.fingerprint_json_file()?),
                    )
                    .await?;
                    let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

                    let fingerprint_hash = fs::must_read_buffered_utf8(
                        &profile_dir.join(bse_unit.fingerprint_hash_file()?),
                    )
                    .await?;

                    // Sanity check that the fingerprint hashes match.
                    if fingerprint.fingerprint_hash() != fingerprint_hash {
                        bail!("fingerprint hash mismatch");
                    }

                    fingerprint
                };

                SavedUnit::BuildScriptExecution(
                    BuildScriptOutputFiles {
                        fingerprint,
                        out_dir_files,
                        stdout,
                        stderr,
                    },
                    bse_unit.clone(),
                )
            }
        };

        let unit_cache_path = cache_path.try_join_file(format!("{}.json", unit_info.unit_hash))?;
        fs::write(&unit_cache_path, serde_json::to_string_pretty(&saved)?).await?;
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
enum SavedUnit {
    LibraryCrate(LibraryFiles, LibraryCrateUnitPlan),
    BuildScriptCompilation(BuildScriptCompiledFiles, BuildScriptCompilationUnitPlan),
    BuildScriptExecution(BuildScriptOutputFiles, BuildScriptExecutionUnitPlan),
}

/// Libraries are usually associated with 7 files:
///
/// - 2 output files (an `.rmeta` and an `.rlib`)
/// - 1 rustc dep-info (`.d`) file in the `deps` folder
/// - 4 files in the fingerprint directory
///   - An `EncodedDepInfo` file
///   - A fingerprint hash
///   - A fingerprint JSON
///   - An invoked timestamp
///
/// Of these files, the fingerprint hash, fingerprint JSON, and invoked
/// timestamp are all reconstructed from fingerprint information during
/// restoration.
#[derive(Debug, Serialize, Deserialize)]
struct LibraryFiles {
    /// These files come from the build plan's `outputs` field.
    // TODO: Can we specify this even more narrowly (e.g. with an `rmeta` and
    // `rlib` field)? I know there are other possible output files (e.g. `.so`
    // for proc macros on Linux and `.dylib` for something on macOS), but I
    // don't know what the enumerated list is.
    output_files: Vec<SavedFile>,
    /// This file is always at a known path in
    /// `deps/{package_name}-{unit_hash}.d`.
    dep_info_file: dep_info2::DepInfo,
    /// This information is parsed from the initial fingerprint created after
    /// the build, and is used to dynamically reconstruct fingerprints on
    /// restoration.
    fingerprint: Fingerprint,
    /// This file is always at a known path in
    /// `.fingerprint/{package_name}-{unit_hash}/dep-lib-{crate_name}`. It can
    /// be safely relocatably copied because the `EncodedDepInfo` struct only
    /// ever contains relative file path information (note that deps always have
    /// a `DepInfoPathType`, which is either `PackageRootRelative` or
    /// `BuildRootRelative`)[^1].
    ///
    /// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/dep_info.rs#L112
    #[serde(with = "base64")]
    encoded_dep_info_file: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BuildScriptCompiledFiles {
    /// This field contains the contents of the compiled build script program at
    /// `build_script_{build_script_entrypoint}-{build_script_compilation_unit_hash}`
    /// and hard linked at `build-script-{build_script_entrypoint}`.
    ///
    /// We need both of these files: the hard link is the file that's actually
    /// executed in the build plan, but the full path with the unit hash is the
    /// file that's tracked by the fingerprint.
    #[serde(with = "base64")]
    compiled_program: Vec<u8>,
    /// This is the path to the rustc dep-info file in the build directory.
    dep_info_file: dep_info2::DepInfo,
    /// This fingerprint is stored in `.fingerprint`, and is used to derive the
    /// timestamp, fingerprint hash file, and fingerprint JSON file.
    fingerprint: Fingerprint,
    /// This `EncodedDepInfo` (i.e. Cargo dep-info) file is stored in
    /// `.fingerprint`, and is directly saved and restored.
    #[serde(with = "base64")]
    encoded_dep_info_file: Vec<u8>,
}

// Note that we don't save
// `{profile_dir}/.fingerprint/{package_name}-{unit_hash}/root-output` because
// it is fully reconstructible from the workspace and the unit plan.
#[derive(Debug, Serialize, Deserialize)]
struct BuildScriptOutputFiles {
    out_dir_files: Vec<SavedFile>,
    stdout: build_script2::BuildScriptOutput,
    #[serde(with = "base64")]
    stderr: Vec<u8>,
    fingerprint: Fingerprint,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedFile {
    path: path2::QualifiedPath,
    #[serde(with = "base64")]
    contents: Vec<u8>,
    executable: bool,
}

mod base64 {
    use serde::{Deserialize, Serialize};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        let base64 = base64::encode(v);
        String::serialize(&base64, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let base64 = String::deserialize(d)?;
        base64::decode(base64.as_bytes()).map_err(|e| serde::de::Error::custom(e))
    }
}
