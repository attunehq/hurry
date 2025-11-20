use std::{collections::HashMap, fmt::Debug, path::PathBuf};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{self, Context, OptionExt as _, bail, eyre},
};
use derive_more::{Debug as DebugExt, Display};
use itertools::Itertools as _;
use scopeguard::defer;
use serde::{Deserialize, Serialize};
use tap::{Conv as _, Pipe as _, Tap as _, TapFallible as _};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace};
use uuid::Uuid;

use crate::{
    cargo::{
        self, BuildPlan, BuildScriptOutput, CargoBuildArguments, CargoCompileMode, Profile,
        RustcArguments, RustcMetadata, RustcTarget,
    },
    fs, mk_rel_dir, mk_rel_file,
    path::{
        AbsDirPath, AbsFilePath, JoinWith as _, RelDirPath, RelFilePath, RelativeTo as _,
        TryJoinWith as _,
    },
};
use clients::courier::v1 as courier;

/// The Cargo workspace of a build.
///
/// Workspaces contain all of the information needed to unambiguously specify
/// the files in a build. Note that workspaces are constructed with a specific
/// invocation in mind, since we parse some of its configuration fields from a
/// build invocation's arguments.
#[derive(Clone, Eq, PartialEq, Hash, DebugExt, Display, Serialize, Deserialize)]
#[display("{root}")]
pub struct Workspace {
    /// The root directory of the workspace.
    pub root: AbsDirPath,

    /// The build directory of the workspace.
    ///
    /// Usually `target` unless user-configured.
    ///
    /// We use the "build dir" terminology from Cargo's upcoming split between
    /// final and intermediate build artifacts[^1].
    ///
    /// [^1]: https://github.com/rust-lang/cargo/issues/6790
    pub build_dir: AbsDirPath,

    /// The $CARGO_HOME value.
    pub cargo_home: AbsDirPath,

    /// The build profile of this workspace invocation.
    pub profile: Profile,

    pub target_arch: RustcTarget,
}

impl Workspace {
    /// Create a workspace by parsing `cargo metadata` from the given directory.
    #[instrument(name = "Workspace::from_argv_in_dir")]
    pub async fn from_argv_in_dir(
        path: &AbsDirPath,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<Self> {
        let args = args.as_ref();

        let (root, build_dir) = {
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
            let workspace_root = root.clone();
            move || home::cargo_home_with_cwd(workspace_root.as_std_path())
        })
        .await
        .context("join background task")?
        .context("get $CARGO_HOME")?
        .pipe(AbsDirPath::try_from)
        .context("parse path as utf8")?;

        let profile = args.profile().map(Profile::from).unwrap_or(Profile::Debug);
        let target_arch = args.target();

        Ok(Self {
            root,
            build_dir,
            cargo_home,
            profile,
            target_arch,
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

    /// Get the profile directory for intermediate build artifacts built for the
    /// host architecture.
    // TODO: Remove this once the migration is complete.
    #[deprecated = "Use unit_profile_dir() instead"]
    pub fn host_profile_dir(&self) -> AbsDirPath {
        self.build_dir
            .try_join_dir(self.profile.as_str())
            .expect("profile should be valid directory name")
    }

    /// Get the profile directory for intermediate build artifacts built for the
    /// target architecture.
    ///
    /// When `--target` is set, this is different from the host profile
    /// directory (even if the target architecture is the same as the host
    /// architecture!), and library and build script execution artifacts are
    /// stored in the target profile directory.
    // TODO: Remove this once the migration is complete.
    #[deprecated = "Use unit_profile_dir() instead"]
    pub fn target_profile_dir(&self) -> AbsDirPath {
        match &self.target_arch {
            RustcTarget::Specified(target_arch) => self
                .build_dir
                .try_join_dirs(vec![target_arch.as_str(), self.profile.as_str()])
                .expect("target arch and profile should be valid directory names"),
            RustcTarget::ImplicitHost => self.host_profile_dir(),
        }
    }

    /// Get the intermediate build artifacts directory for a specific unit.
    ///
    /// ## Cross-Compilation Directory Structure
    ///
    /// When cross-compiling with `--target <triple>`, Cargo uses a complex
    /// directory structure that separates host and target artifacts:
    ///
    /// ```not_rust
    /// target/
    /// ├── <triple>/                    ← Target platform artifacts
    /// │   └── <profile>/
    /// │       ├── deps/                ← Compiled libraries (.rlib, .so, etc.)
    /// │       ├── build/               ← Build script OUTPUT directories
    /// │       │   └── pkg-hash/
    /// │       │       └── out/         ← Files generated by build scripts
    /// │       └── .fingerprint/        ← Fingerprints for:
    /// │           ├── pkg-hash1/       ← - Library compilation
    /// │           └── pkg-hash2/       ← - Build script EXECUTION
    /// └── <profile>/                   ← Host platform artifacts
    ///     ├── build/                   ← Build script BINARIES
    ///     │   └── pkg-hash/
    ///     │       └── build-script-*   ← Compiled build script executables
    ///     └── .fingerprint/            ← Fingerprints for:
    ///         └── pkg-hash/            ← - Build script COMPILATION
    /// ```
    ///
    /// ### The Confusing Part: Build Scripts
    ///
    /// Build scripts have a split personality during cross-compilation:
    ///
    /// 1. **Compilation** (host): The build script itself is a Rust program
    ///    that must run on the build machine (host). Its compiled binary and
    ///    compilation fingerprint live in `target/<profile>/` (no triple).
    ///
    /// 2. **Execution** (target): When the build script runs, it generates
    ///    files for the target platform. Its output directory and execution
    ///    fingerprint live in `target/<triple>/<profile>/`.
    ///
    /// This means for a package like `serde` with a build script:
    /// - Binary: `target/debug/build/serde-{hash1}/build-script-build`
    /// - Binary fingerprint: `target/debug/.fingerprint/serde-{hash1}/`
    /// - Output directory:
    ///   `target/aarch64-apple-darwin/debug/build/serde-{hash2}/out/`
    /// - Output fingerprint:
    ///   `target/aarch64-apple-darwin/debug/.fingerprint/serde-{hash2}/`
    ///
    /// Notice that `hash1` (compilation) and `hash2` (execution) are different
    /// because they represent different compilation units with different
    /// dependencies and flags.
    ///
    /// This function does not do any special handling for build script
    /// compilation versus execution, but upstream functions (like Cargo's build
    /// plan output) already handle this.
    pub fn unit_profile_dir(&self, unit_info: &UnitPlanInfo) -> AbsDirPath {
        match &unit_info.target_arch {
            RustcTarget::Specified(target_arch) => self
                .build_dir
                .try_join_dirs(vec![target_arch.as_str(), self.profile.as_str()])
                .expect("target arch and build profile should be valid directory names"),
            RustcTarget::ImplicitHost => self
                .build_dir
                .try_join_dir(self.profile.as_str())
                .expect("build profile should be valid directory name"),
        }
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
            .try_join_dir(format!("target.backup.{}", Uuid::new_v4()))?;

        let renamed = fs::rename(&self.build_dir, &temp).await.is_ok();

        defer! {
            if renamed {
                let target = self.build_dir.as_std_path();
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

        // When users pass flags like
        // `--message-format=json`, cargo outputs NDJSON (newline-delimited JSON)
        // where the build plan is one of multiple JSON objects. We try parsing
        // each line until we find one with the `invocations` field.
        //
        // We do this instead of e.g. filtering the `--message-format` field because we
        // think that this is less error-prone. If that changes in the future, let's
        // revisit.
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(plan) = serde_json::from_str::<BuildPlan>(line) {
                return Ok(plan);
            }
        }

        // If we didn't find a valid build plan, return an error with context
        Err(eyre!("no valid build plan found in output"))
            .context("parse build plan")
            .with_section(move || stdout.to_string().header("Stdout:"))
            .with_section(move || {
                String::from_utf8_lossy(&output.stderr)
                    .to_string()
                    .header("Stderr:")
            })
    }

    #[instrument(name = "Workspace::units")]
    pub async fn units(
        &self,
        // TODO: These should just use self.args.
        args: impl AsRef<CargoBuildArguments> + std::fmt::Debug,
    ) -> Result<Vec<UnitPlan>> {
        let rustc = RustcMetadata::from_argv(&self.root, &args)
            .await
            .context("parsing rustc metadata")?;
        trace!(?rustc, "rustc metadata");

        // Note that build plans as a feature are deprecated[^1]. If a stable
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
        let build_plan = self.build_plan(&args).await?;
        trace!(?build_plan, "build plan");

        let mut units: Vec<UnitPlan> = Vec::new();
        for mut invocation in build_plan.invocations {
            trace!(?invocation, "build plan invocation");

            // Skip first-party workspace members. We only cache third-party
            // dependencies. `CARGO_PRIMARY_PACKAGE` is set if the user
            // specifically requested the item to be built[^1]; while it's
            // technically possible for the user to do so for a third-party
            // dependency that's relatively rare (and arguably if they're asking
            // to compile it specifically, it _should_ probably be exempt from
            // cache).
            //
            // [^1]: https://doc.rust-lang.org/cargo/reference/environment-variables.html#:~:text=CARGO_PRIMARY_PACKAGE
            if let Some(v) = invocation.env.get("CARGO_PRIMARY_PACKAGE")
                && v == "1"
            {
                trace!("skipping: first party workspace member");
                continue;
            }

            // But also, `CARGO_PRIMARY_PACKAGE` is not set for execution (as
            // opposed to compilation) units[^1]! So we use this heuristic as a
            // fallback.
            //
            // [^1]: https://doc.rust-lang.org/cargo/reference/environment-variables.html#:~:text=This%20is%20only%20set%20when%20compiling%20the%20package%20(not%20when%20running%20binaries%20or%20tests).
            if invocation
                .cwd
                .pipe(AbsFilePath::try_from)?
                .relative_to(&self.cargo_home)
                .is_err()
            {
                trace!("skipping: package outside of $CARGO_HOME");
                continue;
            }

            // Figure out what kind of unit this invocation is.
            let package_name = invocation.package_name;
            let target_arch = invocation.target_arch;
            // FIXME: To support `patch` and `replace` directives when rewriting
            // fingerprints, we need to know the extern_crate_name for each
            // dependency edge of the unit. It is not sufficient to merely know
            // the declared crate_name of the dependency. But we should be able
            // to parse out an extern_crate_name by parsing the `--extern` flags
            // in the invocation rustc arguments for known library output paths.
            let deps = invocation.deps.into_iter().map(|d| d as u32).collect();
            let unit = if invocation.target_kind == [TargetKind::CustomBuild] {
                match invocation.compile_mode {
                    CargoCompileMode::Build => {
                        // Build scripts always compile to a single program and
                        // a renamed hard link to the same program.
                        //
                        // This is parsed from the `outputs` and `links` paths
                        // in the unit's build plan invocation.
                        //
                        // These file paths must have their mtimes modified to
                        // be later than the fingerprint's invoked timestamp for
                        // the unit to be marked fresh.
                        //
                        // In the actual unit struct, we reconstruct these
                        // values from the build script main entry point module
                        // name. But here, we parse the values out of the build
                        // plan and check that our reconstructions are accurate
                        // as a sanity check.
                        let compiled_program = invocation
                            .outputs
                            .into_iter()
                            // Filter out DWARF debugging files, which Cargo removes
                            // anyway.
                            .filter(|o| !o.ends_with(".dwp") && !o.ends_with(".dSYM"))
                            .map(AbsFilePath::try_from)
                            .exactly_one()
                            .unwrap_or_else(|_| {
                                bail!("build script compilation should produce exactly one output");
                            })?;
                        // Resolve `links`. We can just take the keys because we
                        // know they point to valid target files, and there's
                        // only one target file (the compiled program) that we
                        // care about anyway.
                        let linked_program = invocation
                            .links
                            .keys()
                            .filter(|l| !l.ends_with(".dwp") && !l.ends_with(".dSYM"))
                            .map(AbsFilePath::try_from)
                            .exactly_one()
                            .unwrap_or_else(|_| {
                                bail!("build script compilation should produce exactly one output");
                            })?;
                        // Parse unit hash from file name of
                        // `build_script_{entrypoint_module_name}-{hash}`.
                        //
                        // We could also parse this from `-C extra-filename`.
                        let unit_hash = compiled_program
                            .file_name_str_lossy()
                            .ok_or_eyre("program file has no name")?
                            .rsplit_once('-')
                            .ok_or_eyre("program file has no unit hash")?
                            .1
                            .to_string();
                        let args = RustcArguments::from_iter(invocation.args);
                        let crate_name = args
                            .crate_name()
                            .ok_or_eyre("build script compilation should have a crate name")?
                            .to_string();
                        let src_path = args.src_path().pipe(AbsFilePath::try_from)?;
                        // Sanity check that constructed values match parsed
                        // values.
                        if target_arch != RustcTarget::ImplicitHost {
                            bail!(
                                "build script compilation has specified --target architecture {:?}",
                                target_arch
                            );
                        }
                        let profile_dir = self.host_profile_dir();
                        let bsc_unit = BuildScriptCompilationUnitPlan {
                            info: UnitPlanInfo {
                                unit_hash: unit_hash.into(),
                                package_name,
                                crate_name,
                                target_arch,
                                deps,
                            },
                            src_path,
                        };
                        if bsc_unit.program_file()? != compiled_program.relative_to(&profile_dir)? {
                            bail!("build script program filepath reconstruction mismatch");
                        }
                        if bsc_unit.linked_program_file()?
                            != linked_program.relative_to(&profile_dir)?
                        {
                            bail!(
                                "build script program hard link filepath reconstruction mismatch"
                            );
                        }
                        UnitPlan::BuildScriptCompilation(bsc_unit)
                    }
                    CargoCompileMode::RunCustomBuild => {
                        let program = invocation.program.pipe(AbsFilePath::try_from)?;
                        let out_dir = invocation
                            .env
                            .remove("OUT_DIR")
                            .ok_or_eyre("build script execution should set OUT_DIR")?
                            .pipe(AbsDirPath::try_from)?;
                        let unit_dir = out_dir.parent().ok_or_eyre("OUT_DIR should have parent")?;
                        let unit_hash = unit_dir
                            .file_name_str_lossy()
                            .ok_or_eyre("build script execution directory should have name")?
                            .rsplit_once('-')
                            .ok_or_eyre("build script execution directory should have unit hash")?
                            .1
                            .to_string();
                        // Cargo defines the "crate name" of build script
                        // execution as the crate name of the build script being
                        // executed, which we infer from the name of the
                        // compiled program.
                        let crate_name = program
                            .file_name_str_lossy()
                            .ok_or_eyre("build script program should have name")?
                            .to_string()
                            // This is from Cargo's normalization logic.[^1]
                            //
                            // [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/manifest.rs#L961
                            .replace("-", "_");
                        let build_script_program_name = program
                            .file_name_str_lossy()
                            .ok_or_eyre("build script program should have name")?
                            .to_string();
                        // Sanity check that constructed values match parsed
                        // values.
                        let bse_unit = BuildScriptExecutionUnitPlan {
                            info: UnitPlanInfo {
                                unit_hash: unit_hash.into(),
                                package_name,
                                crate_name,
                                target_arch,
                                deps,
                            },
                            build_script_program_name,
                        };
                        let profile_dir = self.unit_profile_dir(&bse_unit.info);
                        if bse_unit.out_dir()? != out_dir.relative_to(&profile_dir)? {
                            bail!("build script out_dir reconstruction mismatch");
                        }

                        UnitPlan::BuildScriptExecution(bse_unit)
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
                // Sanity check: everything here should be a dependency being
                // compiled.
                if invocation.compile_mode != CargoCompileMode::Build {
                    bail!(
                        "unknown compile mode for dependency: {:?}",
                        invocation.compile_mode
                    );
                }

                // Filter out DWARF debugging files, which Cargo removes anyway.
                // These are created for proc macros and cdylibs on Linux (they
                // are `.so.dwp` files).
                //
                // Note there is no need to resolve `links` for library crates.
                // They are never linked unless they are first-party, and we are
                // skipping first-party crates for now anyway.
                let outputs = invocation
                    .outputs
                    .into_iter()
                    .filter(|o| !o.ends_with(".dwp") && !o.ends_with(".dSYM"))
                    .map(AbsFilePath::try_from)
                    .collect::<Result<Vec<_>>>()?;
                let args = RustcArguments::from_iter(invocation.args);
                let crate_name = args.crate_name().ok_or_eyre("no crate name")?.to_owned();
                let src_path = args.src_path().pipe(AbsFilePath::try_from)?;
                // We could also parse this from `-C extra-filename`.
                let unit_hash = {
                    let compiled_file = outputs.first().ok_or_eyre("no compiled files")?;
                    let filename = compiled_file
                        .file_name()
                        .ok_or_eyre("no filename")?
                        .to_string_lossy();
                    let filename = filename.split_once('.').ok_or_eyre("no extension")?.0;

                    filename
                        .rsplit_once('-')
                        .ok_or_eyre(format!(
                            "no unit hash suffix in filename: {filename} (all files: {outputs:?})"
                        ))?
                        .1
                        .to_string()
                };

                UnitPlan::LibraryCrate(LibraryCrateUnitPlan {
                    info: UnitPlanInfo {
                        unit_hash: unit_hash.into(),
                        package_name,
                        crate_name,
                        target_arch,
                        deps,
                    },
                    src_path,
                    outputs,
                })
            } else {
                bail!("unsupported target kind: {:?}", invocation.target_kind);
            };
            units.push(unit);
        }

        Ok(units)
    }

    #[deprecated = "Use units() instead"]
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
                let target = invocation.target_arch.into();

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
            .conv::<Option<String>>()
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
#[deprecated = "Use units instead"]
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
#[deprecated = "Use units instead"]
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

    /// The target triple from the `--target` flag in the rustc invocation for
    /// the _library crate_ of the artifact, if one was specified.
    ///
    /// Note: build scripts ignore this and always build for the local target.
    pub target: Option<String>,

    /// The profile for the _library crate_ of the artifact.
    ///
    /// Note: build scripts ignore this and always build a set profile.
    pub profile: Profile,
}

#[deprecated = "Use units instead"]
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct BuildScriptDirs {
    pub compiled_dir: AbsDirPath,
    pub output_dir: AbsDirPath,
}

/// A BuiltArtifact represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) after it
/// has been built.
#[deprecated = "Use units instead"]
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

    /// The target triple from the `--target` flag in the rustc invocation for
    /// the _library crate_ of the artifact, if one was specified.
    ///
    /// Note: build scripts ignore this and always build for the local target.
    pub target: Option<String>,

    /// The profile for the _library crate_ of the artifact.
    ///
    /// Note: build scripts ignore this and always build a set profile.
    pub profile: Profile,

    /// Memoized profile directory for the _library crate_ of the artifact.
    ///
    /// This is non-public so that it is accessed through the `profile_dir`
    /// method; this value is only here for memoization and because constructing
    /// it is fallible so we construct it inside `from_key` for convenience.
    ///
    /// The intention of making this a method instead of just allowing access to
    /// the field is to have callers infer that this is an _emergent property_
    /// of `BuiltArtifact` information, rather than a concrete piece of data
    /// that Cargo provides.
    ///
    /// If `target` is specified, this is a folder for that target. If not, this
    /// is the default profile dir for the workspace.
    profile_dir: AbsDirPath,
}

impl BuiltArtifact {
    /// Given an `ArtifactKey`, read the build script output directories on
    /// disk and construct a `BuiltArtifact`.
    #[instrument(name = "BuiltArtifact::from_key")]
    pub async fn from_key(ws: &Workspace, key: ArtifactKey) -> Result<Self> {
        // Read the build script output from the build folders, and parse
        // the output for directives.
        let build_script_output = match &key.build_script_files {
            Some(files) => BuildScriptOutput::from_file(
                ws,
                &key.target.clone().into(),
                &files.output_dir.join(mk_rel_file!("output")),
            )
            .await
            .map(Some)?,
            None => None,
        };

        let profile_dir = match &key.target {
            Some(_) => ws.target_profile_dir(),
            None => ws.host_profile_dir(),
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
            profile: key.profile,
            target: key.target,
            profile_dir,
            build_script_output,
        })
    }

    /// The computed profile directory for the _library crate_ of the artifact.
    pub fn profile_dir(&self) -> &AbsDirPath {
        &self.profile_dir
    }
}

/// This is a newtype for unit hash strings.
#[derive(Debug, Display, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct UnitHash(String);

impl From<String> for UnitHash {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<UnitHash> for String {
    fn from(value: UnitHash) -> Self {
        value.0
    }
}

/// Fields which are shared between all unit plan types.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct UnitPlanInfo {
    /// The directory hash of the unit, which is used to construct the unit's
    /// file directories.
    ///
    /// See the `*_dir` methods on `CompilationFiles`[^1] for details.
    ///
    /// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/build_runner/compilation_files.rs#L117
    pub unit_hash: UnitHash,

    /// The package name of this unit.
    ///
    /// This is used to reconstruct expected output directories. See the `*_dir`
    /// methods on `CompilationFiles`[^1] for details.
    ///
    /// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/build_runner/compilation_files.rs#L117
    pub package_name: String,

    /// The crate name of this unit.
    ///
    /// Note that this is not necessarily the _extern_ crate name, which can be
    /// affected by directives like `replace` and `patch`, and that the crate
    /// name used in fingerprints is the extern crate name[^1], not the
    /// canonical crate name.
    ///
    /// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/fingerprint/mod.rs#L1366
    // FIXME: To properly support `replace` and `patch` directives, we need to
    // also calculate an extern_crate_name for each edge in the dependency
    // graph. Note that this is a per-edge value, not a per-unit value. Perhaps
    // we can derive this from the unit graph?
    pub crate_name: String,

    /// The unit's target architecture, if set.
    ///
    /// When None, this unit is not being compiled with a specific `--target` in
    /// mind, and therefore is being compiled for the host architecture.
    ///
    /// Note that some units (e.g. proc macros, build script compilations, and
    /// dependencies thereof) are compiled for the host architecture even when
    /// `--target` is set to a different architecture. This field already takes
    /// that into account.
    pub target_arch: RustcTarget,

    /// The dependencies of this unit.
    ///
    /// This is parsed from the dependencies in the unit's build plan
    /// invocation. Note that units can depend on arbitrary other units. For
    /// example, build script executions can depend on other build script
    /// executions because of the `links` field[^1] or library crates if they
    /// use those libraries.
    ///
    /// This is used to rewrite the unit's fingerprint on restore by rewriting
    /// the fingerprints in the `deps` field.
    ///
    /// [^1]: https://doc.rust-lang.org/cargo/reference/build-scripts.html#the-links-manifest-key
    // This field is not serialized because indexes may not be valid between
    // build plan invocations, and this value should be parsed from the build
    // plan on every build. This does not impact correctness because the
    // dependencies of a unit already have their hash baked into the unit's
    // hash.[^1]
    //
    // [^1]: https://github.com/attunehq/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/build_runner/compilation_files.rs#L721-L737
    #[serde(skip)]
    pub deps: Vec<u32>,
}

impl UnitPlanInfo {
    /// The dependency artifacts directory, relative to the unit's profile
    /// directory. This is used for library units.
    pub fn deps_dir(&self) -> Result<RelDirPath> {
        Ok(mk_rel_dir!("deps"))
    }

    /// The fingerprint directory, relative to the unit's profile directory.
    /// This is used for all units.
    pub fn fingerprint_dir(&self) -> Result<RelDirPath> {
        mk_rel_dir!(".fingerprint")
            .try_join_dir(format!("{}-{}", self.package_name, self.unit_hash))
    }

    /// The build script directory, relative to the unit's profile directory.
    /// This is used for build script compilation and execution units.
    pub fn build_dir(&self) -> Result<RelDirPath> {
        mk_rel_dir!("build").try_join_dir(format!("{}-{}", self.package_name, self.unit_hash))
    }
}

impl From<UnitPlanInfo> for courier::UnitPlanInfo {
    fn from(value: UnitPlanInfo) -> Self {
        Self::builder()
            .unit_hash(value.unit_hash)
            .package_name(value.package_name)
            .crate_name(value.crate_name)
            .maybe_target_arch(value.target_arch.conv::<Option<String>>())
            .build()
    }
}

/// Mode-specific information about this unit.
///
/// This is similar to an amalgamation of TargetKind[^1] and
/// CompileMode[^2].
///
/// Note that we separate compiling library crates from build scripts (even
/// though they are the same CompileMode) because they store artifacts at
/// different paths in the build directory.
///
/// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/manifest.rs#L215
/// [^2]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/build_config.rs#L171
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum UnitPlan {
    LibraryCrate(LibraryCrateUnitPlan),
    BuildScriptCompilation(BuildScriptCompilationUnitPlan),
    BuildScriptExecution(BuildScriptExecutionUnitPlan),
}

impl UnitPlan {
    pub fn info(&self) -> &UnitPlanInfo {
        match self {
            UnitPlan::LibraryCrate(plan) => &plan.info,
            UnitPlan::BuildScriptCompilation(plan) => &plan.info,
            UnitPlan::BuildScriptExecution(plan) => &plan.info,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LibraryCrateUnitPlan {
    pub info: UnitPlanInfo,
    pub src_path: AbsFilePath,
    pub outputs: Vec<AbsFilePath>,
}

impl LibraryCrateUnitPlan {
    pub fn dep_info_file(&self) -> Result<RelFilePath> {
        self.info.deps_dir()?.try_join_file(format!(
            "{}-{}.d",
            self.info.crate_name, self.info.unit_hash
        ))
    }

    pub fn encoded_dep_info_file(&self) -> Result<RelFilePath> {
        self.info
            .fingerprint_dir()?
            .try_join_file(format!("dep-lib-{}", self.info.crate_name))
    }

    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        self.info
            .fingerprint_dir()?
            .try_join_file(format!("lib-{}.json", self.info.crate_name))
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        self.info
            .fingerprint_dir()?
            .try_join_file(format!("lib-{}", self.info.crate_name))
    }
}

impl TryFrom<LibraryCrateUnitPlan> for courier::LibraryCrateUnitPlan {
    type Error = eyre::Report;

    fn try_from(value: LibraryCrateUnitPlan) -> Result<Self> {
        Self::builder()
            .info(value.info)
            .src_path(serde_json::to_string(&value.src_path)?)
            .outputs(
                value
                    .outputs
                    .into_iter()
                    .map(|p| Result::<_>::Ok(serde_json::to_string(&p)?.into()))
                    .try_collect::<_, Vec<_>, _>()?,
            )
            .build()
            .pipe(Ok)
    }
}

impl TryFrom<BuildScriptCompilationUnitPlan> for courier::BuildScriptCompilationUnitPlan {
    type Error = eyre::Report;

    fn try_from(value: BuildScriptCompilationUnitPlan) -> Result<Self> {
        Self::builder()
            .info(value.info)
            .src_path(serde_json::to_string(&value.src_path)?)
            .build()
            .pipe(Ok)
    }
}

impl TryFrom<BuildScriptExecutionUnitPlan> for courier::BuildScriptExecutionUnitPlan {
    type Error = eyre::Report;

    fn try_from(value: BuildScriptExecutionUnitPlan) -> Result<Self> {
        Self::builder()
            .info(value.info)
            .build_script_program_name(value.build_script_program_name)
            .build()
            .pipe(Ok)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildScriptCompilationUnitPlan {
    pub info: UnitPlanInfo,

    /// The path to the build script's main entrypoint source file. This is
    /// usually `build.rs` within the package's source code, but can vary if
    /// the package author sets `package.build` in the package's
    /// `Cargo.toml`, which changes the build script's name[^1].
    ///
    /// This is parsed from the rustc invocation arguments in the unit's
    /// build plan invocation.
    ///
    /// This is used to rewrite the build script compilation's fingerprint
    /// on restore.
    ///
    /// [^1]: https://doc.rust-lang.org/cargo/reference/manifest.html#the-build-field
    pub src_path: AbsFilePath,
}

impl BuildScriptCompilationUnitPlan {
    fn entrypoint_module_name(&self) -> Result<String> {
        let src_path_filename = self
            .src_path
            .file_name_str_lossy()
            .ok_or_eyre("build script source path has no name")?;
        Ok(src_path_filename
            .strip_suffix(".rs")
            .ok_or_eyre("build script source path has no `.rs` extension")?
            .to_string())
    }

    /// Build scripts always compile to a single program and a renamed hard
    /// link to the same program.
    ///
    /// This is parsed from the `outputs` and `links` paths in the unit's
    /// build plan invocation.
    ///
    /// These file paths must have their mtimes modified to be later than
    /// the fingerprint's invoked timestamp for the unit to be marked fresh.
    pub fn program_file(&self) -> Result<RelFilePath> {
        self.info.build_dir()?.try_join_file(format!(
            "build_script_{}-{}",
            self.entrypoint_module_name()?.replace("-", "_"),
            self.info.unit_hash
        ))
    }

    pub fn linked_program_file(&self) -> Result<RelFilePath> {
        self.info
            .build_dir()?
            .try_join_file(format!("build-script-{}", self.entrypoint_module_name()?))
    }

    pub fn dep_info_file(&self) -> Result<RelFilePath> {
        self.info.build_dir()?.try_join_file(format!(
            "build_script_{}-{}.d",
            self.entrypoint_module_name()?.replace("-", "_"),
            self.info.unit_hash
        ))
    }

    pub fn encoded_dep_info_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "dep-build-script-build-script-{}",
            self.entrypoint_module_name()?
        ))
    }

    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "build-script-build-script-{}.json",
            self.entrypoint_module_name()?
        ))
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "build-script-build-script-{}",
            self.entrypoint_module_name()?
        ))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildScriptExecutionUnitPlan {
    // Note that we don't save src_path for build script execution because this
    // field is always set to `""` in the fingerprint for build script execution
    // units[^1].
    //
    // [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/fingerprint/mod.rs#L1665
    pub info: UnitPlanInfo,

    /// The entrypoint module name of the compiled build script program after
    /// linkage (i.e. using the original build script name, which is what Cargo
    /// uses to name the execution unit files).
    pub build_script_program_name: String,
}

impl BuildScriptExecutionUnitPlan {
    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "run-build-script-{}.json",
            self.build_script_program_name
        ))
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "run-build-script-{}",
            self.build_script_program_name
        ))
    }

    pub fn out_dir(&self) -> Result<RelDirPath> {
        Ok(self.info.build_dir()?.join(mk_rel_dir!("out")))
    }

    pub fn stdout_file(&self) -> Result<RelFilePath> {
        Ok(self.info.build_dir()?.join(mk_rel_file!("output")))
    }

    pub fn stderr_file(&self) -> Result<RelFilePath> {
        Ok(self.info.build_dir()?.join(mk_rel_file!("stderr")))
    }

    pub fn root_output_file(&self) -> Result<RelFilePath> {
        Ok(self.info.build_dir()?.join(mk_rel_file!("root-output")))
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

    #[tokio::test]
    async fn build_plan_with_message_format_json() {
        // When --message-format=json is passed, cargo outputs NDJSON
        // (newline-delimited JSON) where the build plan is one of multiple
        // JSON objects. We should still be able to parse it.
        let args = CargoBuildArguments::from_iter(vec!["--message-format=json-render-diagnostics"]);
        let workspace = Workspace::from_argv(&args)
            .await
            .expect("should open workspace");

        let plan = workspace
            .build_plan(&args)
            .await
            .expect("should parse build plan from NDJSON output");

        // Basic sanity checks that we got a valid build plan
        assert!(!plan.invocations.is_empty(), "should have invocations");
        assert!(!plan.inputs.is_empty(), "should have inputs");
    }
}
