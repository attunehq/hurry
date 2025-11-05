use std::collections::HashMap;

use cargo_metadata::TargetKind;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use scopeguard::defer;
use tracing::{debug, instrument, trace};
use uuid::Uuid;

use crate::{
    cargo::{
        ArtifactKey, ArtifactPlan, BuildPlan, BuildScriptDirs, CargoBuildArguments,
        CargoCompileMode, Profile, Workspace,
    },
    cross::{self, CrossConfigGuard},
    path::{AbsDirPath, AbsFilePath},
};

/// Converts Docker container paths to host paths.
///
/// Cross's build plan returns paths relative to the container's filesystem
/// (e.g., "/target/debug/..."). We need to convert these to the host's
/// workspace target directory.
fn convert_container_path_to_host(path: &str, workspace: &Workspace) -> String {
    if let Some(suffix) = path.strip_prefix("/target") {
        format!("{}{}", workspace.target.as_std_path().display(), suffix)
    } else {
        path.to_string()
    }
}

/// Get the build plan by running `cross build --build-plan` with the
/// provided arguments.
///
/// This ensures the build plan is generated in the same environment (host or
/// Docker container) where cross will actually perform the compilation.
#[instrument(name = "cross::build_plan")]
async fn build_plan(
    workspace: &crate::cargo::Workspace,
    args: impl AsRef<CargoBuildArguments> + std::fmt::Debug,
) -> Result<BuildPlan> {
    // Set up Cross.toml to allow RUSTC_BOOTSTRAP passthrough.
    // This guard will restore the original state when dropped.
    let mut config_guard = CrossConfigGuard::setup(workspace.root.as_std_path()).await?;

    // Running `cargo build --build-plan` deletes a bunch of items in the `target`
    // directory. To work around this we temporarily move `target` -> run
    // the build plan -> move it back. If the rename fails (e.g., permissions,
    // cross-device), we proceed without it; this will then have the original issue
    // but at least won't break the build.
    let temp = workspace
        .root
        .as_std_path()
        .join(format!("target.backup.{}", Uuid::new_v4()));

    let renamed = tokio::fs::rename(workspace.target.as_std_path(), &temp)
        .await
        .is_ok();

    defer! {
        if renamed {
            let target = workspace.target.as_std_path();
            #[allow(clippy::disallowed_methods, reason = "cannot use async in defer")]
            let _ = std::fs::remove_dir_all(target);
            #[allow(clippy::disallowed_methods, reason = "cannot use async in defer")]
            let _ = std::fs::rename(&temp, target);
        }
    }

    let mut build_args = args.as_ref().to_argv();
    build_args.extend([
        String::from("--build-plan"),
        String::from("-Z"),
        String::from("unstable-options"),
    ]);
    let output = cross::invoke_output("build", build_args, [("RUSTC_BOOTSTRAP", "1")])
        .await
        .context("run cross command")?;

    // Restore Cross.toml before processing the output
    config_guard
        .restore()
        .await
        .context("restoring Cross.toml")?;

    serde_json::from_slice::<BuildPlan>(&output.stdout)
        .context("parse build plan")
        .with_section(move || {
            String::from_utf8_lossy(&output.stdout)
                .to_string()
                .header("Stdout:")
        })
        .with_section(move || {
            String::from_utf8_lossy(&output.stderr)
                .to_string()
                .header("Stderr:")
        })
}

/// Compute the artifact plan for a cross build.
///
/// This is the cross-specific version of `Workspace::artifact_plan` that ensures
/// build plans and rustc metadata are generated in the same environment where
/// cross will actually perform the compilation.
#[instrument(name = "cross::artifact_plan")]
pub async fn artifact_plan(
    workspace: &crate::cargo::Workspace,
    profile: &Profile,
    args: impl AsRef<CargoBuildArguments> + std::fmt::Debug,
) -> Result<ArtifactPlan> {
    let rustc = crate::cargo::RustcMetadata::from_argv(&workspace.root, &args)
        .await
        .context("parsing rustc metadata")?;
    trace!(?rustc, "rustc metadata");

    // Note that build plans as a feature are _deprecated_, although their
    // removal has not occurred in the last 6 years[^1]. If a stable
    // alternative comes along, we should migrate.
    //
    // An alternative is the `--unit-graph` flag, which is unstable but not
    // deprecated[^2]. Unfortunately, unit graphs do not provide information
    // about the `rustc` invocation argv or the unit hash of the build
    // script execution, both of which are necessary to construct the
    // artifact cache key. We could theoretically reconstruct this
    // information using the JSON build messages and RUSTC_WRAPPER
    // invocation recording, but that's way more work for no stronger of a
    // stability guarantee.
    //
    // [^1]: https://github.com/rust-lang/cargo/issues/7614
    // [^2]: https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph

    // From testing locally, it doesn't seem to matter in which order we
    // pass the flags but we pass the user flags first just in case as that
    // seems like it'd follow the principle of least surprise if ordering
    // ever does matter.
    let build_plan = build_plan(workspace, &args).await?;
    trace!(?build_plan, "build plan");

    let mut build_script_index_to_dir = HashMap::new();
    let mut build_script_program_file_to_index = HashMap::new();
    let mut build_script_executions = HashMap::new();
    let mut artifacts = Vec::new();
    for (i, invocation) in build_plan.invocations.iter().cloned().enumerate() {
        trace!(?invocation, "build plan invocation");

        // For each invocation, figure out what kind it is:
        // 1. Compiling a build script.
        // 2. Running a build script.
        // 3. Compiling a dependency.
        // 4. Compiling first-party code (which we skip for caching).
        if invocation.target_kind == [TargetKind::CustomBuild] {
            match invocation.compile_mode {
                CargoCompileMode::Build => {
                    if let Some(output_file) = invocation.outputs.first() {
                        // For build script compilation, we need to know the
                        // directory into which the build script is
                        // compiled and record the compiled program file.

                        // First, we determine the build script compilation
                        // directory. Convert from container path to host path.
                        let host_path = convert_container_path_to_host(output_file, workspace);
                        let output_file = std::path::PathBuf::from(host_path);
                        let out_dir = output_file
                            .parent()
                            .ok_or_eyre("build script output file should have parent directory")?
                            .to_owned();
                        build_script_index_to_dir.insert(i, out_dir);

                        // Second, we record the executable program.
                        for file in invocation.outputs {
                            let host_file = convert_container_path_to_host(&file, workspace);
                            build_script_program_file_to_index.insert(host_file, i);
                        }
                        for (fslink, _orig) in invocation.links {
                            let host_link = convert_container_path_to_host(&fslink, workspace);
                            build_script_program_file_to_index.insert(host_link, i);
                        }
                    } else {
                        bail!(
                            "build script compilation produced no outputs: {:?}",
                            invocation
                        );
                    }
                }
                CargoCompileMode::RunCustomBuild => {
                    // For build script execution, we need to know which
                    // compiled build script is being executed, and where
                    // its outputs are being written.

                    // First, we need to figure out the build script being
                    // executed. We can do this using the program file being
                    // executed. Convert the program path from container to host.
                    let host_program = convert_container_path_to_host(&invocation.program, workspace);
                    let build_script_index = *build_script_program_file_to_index
                        .get(&host_program)
                        .ok_or_eyre("build script should be compiled before execution")?;

                    // Second, we need to determine where its outputs are being written.
                    // Convert OUT_DIR from container path to host path.
                    let out_dir = invocation
                        .env
                        .get("OUT_DIR")
                        .ok_or_eyre("build script execution should set OUT_DIR")?;
                    let host_out_dir = convert_container_path_to_host(out_dir, workspace);

                    build_script_executions.insert(i, (build_script_index, host_out_dir));
                }
                _ => bail!(
                    "unknown compile mode for build script: {:?}",
                    invocation.compile_mode
                ),
            }
        } else if invocation.target_kind == [TargetKind::Bin] {
            // Binaries are _always_ first-party code. Do nothing for now.
            continue;
        } else if invocation.target_kind.contains(&TargetKind::Lib)
            || invocation.target_kind.contains(&TargetKind::RLib)
            || invocation.target_kind.contains(&TargetKind::CDyLib)
            || invocation.target_kind.contains(&TargetKind::ProcMacro)
        {
            // Skip first-party workspace members. We only cache third-party dependencies.
            // `CARGO_PRIMARY_PACKAGE` is set if the user specifically requested the item
            // to be built; while it's technically possible for the user to do so for a
            // third-party dependency that's relatively rare (and arguably if they're asking
            // to compile it specifically, it _should_ probably be exempt from cache).
            let primary = invocation.env.get("CARGO_PRIMARY_PACKAGE");
            if primary.map(|v| v.as_str()) == Some("1") {
                trace!(?invocation, "skipping: first party workspace member");
                continue;
            }

            // Sanity check: everything here should be a dependency being compiled.
            if invocation.compile_mode != CargoCompileMode::Build {
                bail!(
                    "unknown compile mode for dependency: {:?}",
                    invocation.compile_mode
                );
            }

            let mut build_script_execution_index = None;
            for dep_index in &invocation.deps {
                let dep = &build_plan.invocations[*dep_index];
                // This should be sufficient to determine which dependency
                // is the execution of the build script of the current
                // library. There might be other build scripts for the same
                // name and version (but different features), but they won't
                // be listed as a `dep`.
                if dep.target_kind == [TargetKind::CustomBuild]
                    && dep.compile_mode == CargoCompileMode::RunCustomBuild
                    && dep.package_name == invocation.package_name
                    && dep.package_version == invocation.package_version
                {
                    build_script_execution_index = Some(dep_index);
                    break;
                }
            }

            // Convert Docker container paths to host paths.
            let lib_files: Vec<AbsFilePath> = invocation
                .outputs
                .into_iter()
                .map(|f| {
                    let host_path = convert_container_path_to_host(&f, workspace);
                    AbsFilePath::try_from(host_path).context("parsing build plan output file")
                })
                .collect::<Result<Vec<_>>>()?;
            debug!(
                package = invocation.package_name,
                ?lib_files,
                "library output files from build plan"
            );
            let library_crate_compilation_unit_hash = {
                let compiled_file = lib_files.first().ok_or_eyre("no compiled files")?;
                let filename = compiled_file
                    .file_name()
                    .ok_or_eyre("no filename")?
                    .to_string_lossy();
                let filename = filename.split_once('.').ok_or_eyre("no extension")?.0;

                filename
                    .rsplit_once('-')
                    .ok_or_else(|| {
                        eyre!(
                            "no unit hash suffix in filename: {filename} (all files: {lib_files:?})"
                        )
                    })?
                    .1
                    .to_string()
            };
            let build_script = match build_script_execution_index {
                Some(build_script_execution_index) => {
                    let (build_script_index, build_script_output_dir) = build_script_executions
                        .get(build_script_execution_index)
                        .ok_or_eyre(
                            "build script execution should have recorded output directory",
                        )?;
                    // We take the parent because this is always the `/out`
                    // folder of the build script.
                    let build_script_output_dir = AbsDirPath::try_from(build_script_output_dir)?
                        .parent()
                        .ok_or_eyre("build script output directory has no parent")?;
                    let build_script_compiled_dir = build_script_index_to_dir
                        .get(build_script_index)
                        .ok_or_eyre(
                            "build script index should have recorded compilation directory",
                        )?;
                    let build_script_compiled_dir = AbsDirPath::try_from(build_script_compiled_dir)?;
                    let build_script_compilation_unit_hash = {
                        let filename = &build_script_compiled_dir
                            .file_name()
                            .ok_or_eyre("no filename")?
                            .to_string_lossy();

                        filename
                            .rsplit_once('-')
                            .ok_or_eyre("no unit hash suffix")?
                            .1
                            .to_string()
                    };
                    let build_script_output_unit_hash = {
                        let filename = &build_script_output_dir
                            .file_name()
                            .ok_or_eyre("out_dir has no filename")?
                            .to_string_lossy();

                        filename
                            .rsplit_once('-')
                            .ok_or_eyre("no unit hash suffix")?
                            .1
                            .to_string()
                    };
                    Some((
                        BuildScriptDirs {
                            compiled_dir: build_script_compiled_dir,
                            output_dir: build_script_output_dir,
                        },
                        build_script_compilation_unit_hash,
                        build_script_output_unit_hash,
                    ))
                }
                None => None,
            };
            let (
                build_script_files,
                build_script_compilation_unit_hash,
                build_script_execution_unit_hash,
            ) = match build_script {
                Some((
                    build_script_files,
                    build_script_compilation_unit_hash,
                    build_script_execution_unit_hash,
                )) => (
                    Some(build_script_files),
                    Some(build_script_compilation_unit_hash),
                    Some(build_script_execution_unit_hash),
                ),
                None => (None, None, None),
            };

            // Given a dependency being compiled, we need to determine the
            // compiled files, its build script directory, and its build
            // script outputs directory. These are the files that we're
            // going to save for this artifact.
            debug!(
                compiled = ?lib_files,
                build_script = ?build_script_files,
                deps = ?invocation.deps,
                "artifacts to save"
            );
            // Use the `kind` field from the build plan to determine the target.
            // This is cargo's canonical representation of host vs target compilation:
            // - `kind: None` means compiled for host (proc-macros, build scripts)
            // - `kind: Some(target)` means compiled for the specified target
            //
            // Note: This is more reliable than parsing rustc args because cargo
            // always sets this field correctly regardless of whether cross or cargo
            // invoked the build.
            let target = invocation.kind.clone();
            debug!(
                package = invocation.package_name,
                ?target,
                "determining artifact target from build plan kind field"
            );

            artifacts.push(ArtifactKey {
                package_name: invocation.package_name,
                package_version: invocation.package_version,
                profile: profile.clone(),
                lib_files,
                build_script_files,
                library_crate_compilation_unit_hash,
                build_script_compilation_unit_hash,
                build_script_execution_unit_hash,
                target,
            });

            // TODO: If needed, we could try to read previous build script
            // output from the target directory here to try and supplement
            // information for built crates. I can't imagine why we would
            // need to do that, though.
        } else {
            bail!("unknown target kind: {:?}", invocation.target_kind);
        }
    }

    // The target for the build is the user-provided `--target` flag, or the host
    // target.
    //
    // Note: Individual artifacts may have different targets (some for host, some
    // for the specified target), but this represents the "effective target"
    // of the build for cache keying purposes.
    let target = args
        .as_ref()
        .target()
        .map(String::from)
        .unwrap_or_else(|| rustc.host_target.clone());

    Ok(ArtifactPlan {
        artifacts,
        target,
        profile: profile.clone(),
    })
}
