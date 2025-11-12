use std::{
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
    eyre::{Context, OptionExt as _, bail},
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
        QualifiedPath, Workspace, build_script2, dep_info2, path2,
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
    /// Base URL for the Courier instance.
    #[arg(
        long = "hurry-courier-url",
        env = "HURRY_COURIER_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{courier_url}")]
    courier_url: Url,

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

    // TODO: Restore artifacts.

    // Run build.
    cargo::invoke("build", &options.argv)
        .await
        .context("build with cargo")?;

    // Save artifacts.
    for unit in unit_plan {
        // Support cross-compilation. Note that some library crates may be built
        // on the host even when `--target` is set (e.g. proc macros and build
        // script dependencies). This field already correctly sets the
        // `target_arch` value taking that into account.
        let profile_dir = match &unit.target_arch {
            Some(_) => workspace.target_profile_dir(),
            None => workspace.host_profile_dir(),
        };

        let files = match &unit.mode {
            workspace2::UnitPlanMode::LibraryCrate(
                unit_plan @ LibraryCrateUnitPlan { src_path, outputs },
            ) => {
                let output_files = {
                    let mut output_files = Vec::new();
                    for output_file_path in outputs.into_iter() {
                        let path = path2::QualifiedPath::parse(
                            &workspace,
                            &unit,
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

                let dep_info_file = {
                    let deps_dir = profile_dir.join(mk_rel_dir!("deps"));
                    let dep_info_file_path = deps_dir
                        .try_join_file(format!("{}-{}.d", unit.crate_name, unit.unit_hash))?;
                    dep_info2::DepInfo::from_file(&workspace, &unit, &dep_info_file_path).await?
                };

                let fingerprint_dir = profile_dir.try_join_dirs(&[
                    String::from(".fingerprint"),
                    format!("{}-{}", unit.package_name, unit.unit_hash),
                ])?;

                let encoded_dep_info_file = {
                    let encoded_dep_info_file_path =
                        fingerprint_dir.try_join_file(format!("dep-lib-{}", unit.crate_name))?;
                    fs::must_read_buffered(&encoded_dep_info_file_path).await?
                };

                let fingerprint = {
                    let fingerprint_file_path =
                        fingerprint_dir.try_join_file(format!("lib-{}.json", unit.crate_name))?;
                    let content = fs::must_read_buffered_utf8(&fingerprint_file_path).await?;
                    let fingerprint: Fingerprint = serde_json::from_str(&content)?;

                    let fingerprint_hash_file_path =
                        fingerprint_dir.try_join_file(format!("lib-{}", unit.crate_name))?;
                    let fingerprint_hash =
                        fs::must_read_buffered_utf8(&fingerprint_hash_file_path).await?;

                    // Sanity check that the fingerprint hashes match.
                    if hex::encode(fingerprint.hash_u64().to_le_bytes()) != fingerprint_hash {
                        bail!("fingerprint hash mismatch");
                    }

                    fingerprint
                };

                UnitFiles::LibraryCrate(LibraryFiles {
                    output_files,
                    dep_info_file,
                    fingerprint,
                    encoded_dep_info_file,
                })
            }
            workspace2::UnitPlanMode::BuildScriptCompilation(
                unit_plan @ BuildScriptCompilationUnitPlan {
                    src_path,
                    program,
                    linked_program,
                },
            ) => {
                let compiled_program = fs::must_read_buffered(&program).await?;

                let dep_info_file = {
                    let program_filename = program
                        .file_name_str_lossy()
                        .ok_or_eyre("build script program has no name")?;
                    let dep_info_file_path = program
                        .parent()
                        .ok_or_eyre("build script program not in directory")?
                        .try_join_file(format!("{}.d", program_filename))?;
                    dep_info2::DepInfo::from_file(&workspace, &unit, &dep_info_file_path).await?
                };

                let fingerprint_dir = profile_dir.try_join_dirs(&[
                    String::from(".fingerprint"),
                    format!("{}-{}", unit.package_name, unit.unit_hash),
                ])?;
                let linked_filename = linked_program
                    .file_name_str_lossy()
                    .ok_or_eyre("build script hard link has no name")?;

                let encoded_dep_info_file = {
                    let encoded_dep_info_file_path = fingerprint_dir
                        .try_join_file(format!("dep-build-script-{}", linked_filename))?;
                    fs::must_read_buffered(&encoded_dep_info_file_path).await?
                };

                let fingerprint = {
                    let fingerprint_file_path = fingerprint_dir
                        .try_join_file(format!("build-script-{}.json", linked_filename))?;
                    let content = fs::must_read_buffered_utf8(&fingerprint_file_path).await?;
                    let fingerprint: Fingerprint = serde_json::from_str(&content)?;

                    let fingerprint_hash_file_path = fingerprint_dir
                        .try_join_file(format!("build-script-{}", linked_filename))?;
                    let fingerprint_hash =
                        fs::must_read_buffered_utf8(&fingerprint_hash_file_path).await?;

                    // Sanity check that the fingerprint hashes match.
                    if hex::encode(fingerprint.hash_u64().to_le_bytes()) != fingerprint_hash {
                        bail!("fingerprint hash mismatch");
                    }

                    fingerprint
                };

                UnitFiles::BuildScriptCompilation(BuildScriptCompiledFiles {
                    compiled_program,
                    dep_info_file,
                    encoded_dep_info_file,
                    fingerprint,
                })
            }
            workspace2::UnitPlanMode::BuildScriptExecution(
                unit_plan @ BuildScriptExecutionUnitPlan { program, out_dir },
            ) => {
                let build_script_output_dir = profile_dir.try_join_dirs(&[
                    String::from("build"),
                    format!("{}-{}", unit.package_name, unit.unit_hash),
                ])?;

                let stdout = build_script2::BuildScriptOutput::from_file(
                    &workspace,
                    &unit,
                    &build_script_output_dir.try_join_file("output")?,
                )
                .await?;
                let stderr =
                    fs::must_read_buffered(&build_script_output_dir.try_join_file("stderr")?)
                        .await?;
                let out_dir_files = {
                    let files = fs::walk_files(&out_dir).try_collect::<Vec<_>>().await?;
                    let mut out_dir_files = Vec::new();
                    for file in files {
                        let path =
                            path2::QualifiedPath::parse(&workspace, &unit, file.as_std_path())
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
                    let fingerprint_dir = profile_dir.try_join_dirs(&[
                        String::from(".fingerprint"),
                        format!("{}-{}", unit.package_name, unit.unit_hash),
                    ])?;
                    let program_filename = program
                        .file_name_str_lossy()
                        .ok_or_eyre("build script program has no name")?;

                    let fingerprint_file_path = fingerprint_dir
                        .try_join_file(format!("run-build-script-{}.json", program_filename))?;
                    let content = fs::must_read_buffered_utf8(&fingerprint_file_path).await?;
                    let fingerprint: Fingerprint = serde_json::from_str(&content)?;

                    let fingerprint_hash_file_path = fingerprint_dir
                        .try_join_file(format!("run-build-script-{}", program_filename))?;
                    let fingerprint_hash =
                        fs::must_read_buffered_utf8(&fingerprint_hash_file_path).await?;

                    // Sanity check that the fingerprint hashes match.
                    if hex::encode(fingerprint.hash_u64().to_le_bytes()) != fingerprint_hash {
                        bail!("fingerprint hash mismatch");
                    }

                    fingerprint
                };

                UnitFiles::BuildScriptExecution(BuildScriptOutputFiles {
                    fingerprint,
                    out_dir_files,
                    stdout,
                    stderr,
                })
            }
        };

        let unit_hash = unit.unit_hash.clone();
        let saved = SavedUnit { files, unit };
        fs::write(
            &cache_path.try_join_file(format!("{}.json", unit_hash))?,
            serde_json::to_string_pretty(&saved)?,
        )
        .await?;
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedUnit {
    files: UnitFiles,
    unit: workspace2::UnitPlan,
}

#[derive(Debug, Serialize, Deserialize)]
enum UnitFiles {
    LibraryCrate(LibraryFiles),
    BuildScriptCompilation(BuildScriptCompiledFiles),
    BuildScriptExecution(BuildScriptOutputFiles),
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
