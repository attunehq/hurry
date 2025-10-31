use std::{collections::HashMap, fmt::Debug, path::PathBuf};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use derive_more::{Debug as DebugExt, Display};
use scopeguard::defer;
use serde::{Deserialize, Serialize};
use tap::{Pipe as _, Tap as _, TapFallible as _};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace};
use uuid::Uuid;

use crate::{
    cargo::{
        self, BuildPlan, BuildScriptOutput, CargoBuildArguments, CargoCompileMode, Profile,
        QualifiedPath, RustcMetadata, build_plan::RustcInvocationArgument,
    },
    mk_rel_file,
    path::{AbsDirPath, AbsFilePath, JoinWith as _, TryJoinWith as _},
};
use clients::courier::v1::Key;

/// The Cargo workspace of a build.
///
/// Workspaces contain all the paths needed to unambiguously specify the files
/// in a build. Note that workspaces are constructed with a specific profile in
/// mind, which we parse from the build command's arguments.
#[derive(Clone, Eq, PartialEq, Hash, DebugExt, Display, Serialize, Deserialize)]
#[display("{root}")]
pub struct Workspace {
    /// The root directory of the workspace.
    pub root: AbsDirPath,

    /// The target directory in the workspace.
    #[debug(skip)]
    pub target: AbsDirPath,

    /// The $CARGO_HOME value.
    #[debug(skip)]
    pub cargo_home: AbsDirPath,

    /// The build profile of this workspace invocation.
    pub profile: Profile,

    /// The build profile target directory.
    #[debug(skip)]
    pub profile_dir: AbsDirPath,
}

impl Workspace {
    /// Create a workspace by parsing `cargo metadata` from the given directory.
    #[instrument(name = "Workspace::from_argv_in_dir")]
    pub async fn from_argv_in_dir(
        path: &AbsDirPath,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<Self> {
        let args = args.as_ref();

        let (workspace_root, workspace_target) = {
            // TODO: Maybe we should just replicate this logic and perform it
            // statically using filesystem operations instead of shelling out?
            // This costs something on the order of 200ms, which is not
            // _terrible_ but feels much slower than if we just did our own
            // filesystem reads, especially since we don't actually use any of
            // the logic except the paths.
            let manifest_path = args.manifest_path().map(String::from);
            let cmd_current_dir = path.as_std_path().to_path_buf();
            let metadata = spawn_blocking(move || -> Result<_> {
                cargo_metadata::MetadataCommand::new()
                    .tap_mut(|cmd| {
                        if let Some(p) = manifest_path {
                            cmd.manifest_path(p);
                        }
                    })
                    .current_dir(cmd_current_dir)
                    .exec()
                    .context("exec and parse cargo metadata")
            })
            .await
            .context("join task")?
            .tap_ok(|metadata| debug!(?metadata, "cargo metadata"))
            .context("get cargo metadata")?;
            (
                AbsDirPath::try_from(&metadata.workspace_root)
                    .context("parse workspace root as absolute directory")?,
                AbsDirPath::try_from(&metadata.target_directory)
                    .context("parse workspace target as absolute directory")?,
            )
        };

        let cargo_home = spawn_blocking({
            let workspace_root = workspace_root.clone();
            move || home::cargo_home_with_cwd(workspace_root.as_std_path())
        })
        .await
        .context("join background task")?
        .context("get $CARGO_HOME")?
        .pipe(AbsDirPath::try_from)
        .context("parse path as utf8")?;

        let profile = args.profile().map(Profile::from).unwrap_or(Profile::Debug);
        let profile_dir = workspace_target.try_join_dir(profile.as_str())?;

        Ok(Self {
            root: workspace_root,
            target: workspace_target,
            cargo_home,
            profile,
            profile_dir,
        })
    }

    /// Create a workspace from the current working directory.
    ///
    /// Convenience method that calls `from_argv_in_dir`
    /// using the current working directory as the workspace root.
    #[instrument(name = "Workspace::from_argv")]
    pub async fn from_argv(args: impl AsRef<CargoBuildArguments> + Debug) -> Result<Self> {
        let pwd = AbsDirPath::current().context("get working directory")?;
        Self::from_argv_in_dir(&pwd, args).await
    }

    /// Get the build plan by running `cargo build --build-plan` with the
    /// provided arguments.
    #[instrument(name = "Workspace::build_plan")]
    async fn build_plan(
        &self,
        args: impl AsRef<CargoBuildArguments> + std::fmt::Debug,
    ) -> Result<BuildPlan> {
        // Running `cargo build --build-plan` deletes a bunch of items in the `target`
        // directory. To work around this we temporarily move `target` -> run
        // the build plan -> move it back. If the rename fails (e.g., permissions,
        // cross-device), we proceed without it; this will then have the original issue
        // but at least won't break the build.
        let temp = self
            .root
            .as_std_path()
            .join(format!("target.backup.{}", Uuid::new_v4()));

        let renamed = tokio::fs::rename(self.target.as_std_path(), &temp)
            .await
            .is_ok();

        defer! {
            if renamed {
                let target = self.target.as_std_path();
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
        let output = cargo::invoke_output("build", build_args, [("RUSTC_BOOTSTRAP", "1")])
            .await
            .context("run cargo command")?;
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

    #[instrument(name = "Workspace::artifact_plan")]
    pub async fn artifact_plan(
        &self,
        profile: &Profile,
        args: impl AsRef<CargoBuildArguments> + std::fmt::Debug,
    ) -> Result<ArtifactPlan> {
        let rustc = RustcMetadata::from_argv(&self.root, &args)
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
        let build_plan = self.build_plan(&args).await?;
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
                            // directory.
                            let output_file = PathBuf::from(output_file);
                            let out_dir = output_file
                                .parent()
                                .ok_or_eyre(
                                    "build script output file should have parent directory",
                                )?
                                .to_owned();
                            build_script_index_to_dir.insert(i, out_dir);

                            // Second, we record the executable program.
                            for file in invocation.outputs {
                                build_script_program_file_to_index.insert(file, i);
                            }
                            for (fslink, _orig) in invocation.links {
                                build_script_program_file_to_index.insert(fslink, i);
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
                        // executed.
                        let build_script_index = *build_script_program_file_to_index
                            .get(&invocation.program)
                            .ok_or_eyre("build script should be compiled before execution")?;

                        // Second, we need to determine where its outputs are being written.
                        let out_dir = invocation
                            .env
                            .get("OUT_DIR")
                            .ok_or_eyre("build script execution should set OUT_DIR")?
                            .clone();

                        build_script_executions.insert(i, (build_script_index, out_dir));
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

                let lib_files: Vec<AbsFilePath> = invocation
                    .outputs
                    .into_iter()
                    .map(|f| AbsFilePath::try_from(f).context("parsing build plan output file"))
                    .collect::<Result<Vec<_>>>()?;
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
                        let build_script_output_dir =
                            AbsDirPath::try_from(build_script_output_dir)?
                                .parent()
                                .ok_or_eyre("build script output directory has no parent")?;
                        let build_script_compiled_dir = build_script_index_to_dir
                            .get(build_script_index)
                            .ok_or_eyre(
                                "build script index should have recorded compilation directory",
                            )?;
                        let build_script_compiled_dir =
                            AbsDirPath::try_from(build_script_compiled_dir)?;
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
                artifacts.push(ArtifactKey {
                    package_name: invocation.package_name,
                    package_version: invocation.package_version,
                    lib_files,
                    build_script_files,
                    library_crate_compilation_unit_hash,
                    build_script_compilation_unit_hash,
                    build_script_execution_unit_hash,
                });

                // TODO: If needed, we could try to read previous build script
                // output from the target directory here to try and supplement
                // information for built crates. I can't imagine why we would
                // need to do that, though.
            } else {
                bail!("unknown target kind: {:?}", invocation.target_kind);
            }
        }

        // Extract the target from the rustc invocations. All artifacts in a single
        // build use the same target, so we just need to find the first one.
        let target = build_plan
            .invocations
            .iter()
            .find_map(|invocation| {
                invocation.args.iter().find_map(|arg| match arg {
                    RustcInvocationArgument::Target(target) => Some(target.clone()),
                    _ => None,
                })
            })
            .unwrap_or_else(|| rustc.host_target.clone());

        Ok(ArtifactPlan {
            artifacts,
            target,
            profile: profile.clone(),
        })
    }
}

/// An ArtifactPlan represents the collection of information known about the
/// artifacts for a build statically at compile-time.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct ArtifactPlan {
    pub profile: Profile,
    pub target: String,

    pub artifacts: Vec<ArtifactKey>,
}

/// An ArtifactKey represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) statically
/// at plan-time.
///
/// In particular, this information does _not_ include information derived from
/// compiling and running the build script, such as `rustc` flags from build
/// script output directives.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct ArtifactKey {
    // Partial artifact key information. Note that this is only derived from the
    // build plan, and therefore is missing essential information (e.g. `rustc`
    // flags from build script output directives) that can only be determined
    // interactively.
    //
    // TODO: There are more fields here that we can know from the planning stage
    // that need to be added (e.g. features).
    pub package_name: String,
    pub package_version: String,

    // Artifact folders to save and restore.
    pub lib_files: Vec<AbsFilePath>,
    pub build_script_files: Option<BuildScriptDirs>,

    // Unit hashes.
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct BuildScriptDirs {
    pub compiled_dir: AbsDirPath,
    pub output_dir: AbsDirPath,
}

/// A BuiltArtifact represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) after it
/// has been built.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct BuiltArtifact {
    pub package_name: String,
    pub package_version: String,

    pub lib_files: Vec<AbsFilePath>,
    pub build_script_files: Option<BuildScriptDirs>,

    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,

    // TODO: Should these all be in a larger `BuildScript` struct that includes
    // the files, unit hashes, and output? It's a little silly to all have them
    // be separately optional, as if we could have some fields but not others.
    pub build_script_output: Option<BuildScriptOutput>,
}

impl BuiltArtifact {
    /// Given an `ArtifactKey`, read the build script output directories on
    /// disk and construct a `BuiltArtifact`.
    #[instrument(name = "BuiltArtifact::from_key")]
    pub async fn from_key(ws: &Workspace, key: ArtifactKey) -> Result<Self> {
        // Read the build script output from the build folders, and parse
        // the output for directives.
        let build_script_output = match &key.build_script_files {
            Some(build_script_files) => {
                let bso = BuildScriptOutput::from_file(
                    ws,
                    &build_script_files.output_dir.join(mk_rel_file!("output")),
                )
                .await?;
                Some(bso)
            }
            None => None,
        };

        // TODO: Use this later to reconstruct the rustc invocation, and use all
        // of this information to fully construct the cache key.
        Ok(BuiltArtifact {
            package_name: key.package_name,
            package_version: key.package_version,

            lib_files: key.lib_files,
            build_script_files: key.build_script_files,

            library_crate_compilation_unit_hash: key.library_crate_compilation_unit_hash,
            build_script_compilation_unit_hash: key.build_script_compilation_unit_hash,
            build_script_execution_unit_hash: key.build_script_execution_unit_hash,
            build_script_output,
        })
    }
}

/// A content hash of a library unit's artifacts.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize)]
pub struct LibraryUnitHash {
    files: Vec<(QualifiedPath, Key)>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
struct LibraryUnitHashOrd<'a>(&'a QualifiedPath);

impl<'a> LibraryUnitHashOrd<'a> {
    fn discriminant(&self) -> u64 {
        match &self.0 {
            QualifiedPath::Rootless(_) => 0,
            QualifiedPath::RelativeTargetProfile(_) => 1,
            QualifiedPath::RelativeCargoHome(_) => 2,
            QualifiedPath::Absolute(_) => 3,
        }
    }
}

impl<'a> Ord for LibraryUnitHashOrd<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.0, &other.0) {
            (QualifiedPath::Rootless(a), QualifiedPath::Rootless(b)) => a.cmp(b),
            (QualifiedPath::RelativeTargetProfile(a), QualifiedPath::RelativeTargetProfile(b)) => {
                a.cmp(b)
            }
            (QualifiedPath::RelativeCargoHome(a), QualifiedPath::RelativeCargoHome(b)) => a.cmp(b),
            (QualifiedPath::Absolute(a), QualifiedPath::Absolute(b)) => a.cmp(b),
            (_, _) => self.discriminant().cmp(&other.discriminant()),
        }
    }
}

impl<'a> PartialOrd for LibraryUnitHashOrd<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LibraryUnitHash {
    /// Construct a library unit hash out of the files in the library unit.
    ///
    /// This constructor always ensures that the files are sorted, so any two
    /// sets of files with the same paths and contents will produce the same
    /// hash.
    pub fn new(mut files: Vec<(QualifiedPath, Key)>) -> Self {
        files.sort_by(|(q1, k1), (q2, k2)| {
            (LibraryUnitHashOrd(q1), k1).cmp(&(LibraryUnitHashOrd(q2), k2))
        });
        Self { files }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq as pretty_assert_eq;

    #[tokio::test]
    async fn build_plan_flag_order_does_not_matter() {
        // This is a relatively basic test to start with; if we find other edge
        // cases we want to test we should add them here (or in a similar test).
        let user_args = ["--release"];
        let tool_args = ["--build-plan", "-Z", "unstable-options"];
        let env = [("RUSTC_BOOTSTRAP", "1")];
        let cmd = "build";

        let args = user_args.iter().chain(tool_args.iter());
        let user_args_first = match cargo::invoke_output(cmd, args, env).await {
            Ok(output) => output.stdout,
            Err(e) => panic!("user args first should succeed: {e}"),
        };

        let args = tool_args.iter().chain(user_args.iter());
        let tool_args_first = match cargo::invoke_output(cmd, args, env).await {
            Ok(output) => output.stdout,
            Err(e) => panic!("tool args first should succeed: {e}"),
        };

        let user_plan = serde_json::from_slice::<BuildPlan>(&user_args_first).unwrap();
        let tool_plan = serde_json::from_slice::<BuildPlan>(&tool_args_first).unwrap();
        pretty_assert_eq!(
            user_plan,
            tool_plan,
            "both orderings should produce same build plan"
        );
    }
}
