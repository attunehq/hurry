use std::{fmt::Debug, time::SystemTime};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt as _, bail, eyre},
};
use derive_more::{Debug as DebugExt, Display};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use tap::{Conv as _, Tap as _, TapFallible as _, TryConv as _};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace};
use uuid::Uuid;

use crate::{
    cargo::{
        self, BuildPlan, BuildScriptCompilationUnitPlan, BuildScriptExecutionUnitPlan,
        CargoBuildArguments, CargoCompileMode, Fingerprint, LibraryCrateUnitPlan, Profile,
        RustcArguments, RustcTarget, RustcTargetPlatform,
    },
    fs, mk_rel_dir,
    path::{AbsDirPath, AbsFilePath, RelDirPath, RelFilePath, RelativeTo as _, TryJoinWith as _},
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

    /// The architecture specified for the build via `--target`.
    pub target_arch: RustcTarget,

    /// The architecture of the host machine.
    pub host_arch: RustcTargetPlatform,
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
            .tap_ok(|metadata| trace!(?metadata, "cargo metadata"))
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
        .try_into()
        .context("parse path as utf8")?;

        let host_arch = {
            let mut cmd = tokio::process::Command::new("cargo");
            cmd.args(["-Z", "unstable-options", "rustc", "--print", "host-tuple"]);
            // This is apparently still unstable[^1] when invoked as `cargo
            // rustc`.
            //
            // [^1]: https://github.com/rust-lang/cargo/issues/9357
            cmd.env("RUSTC_BOOTSTRAP", "1");
            let output = cmd.output().await.context("run rustc")?;
            if !output.status.success() {
                return Err(eyre!("invoke rustc"))
                    .with_section(|| {
                        String::from_utf8_lossy(&output.stdout)
                            .to_string()
                            .header("Stdout:")
                    })
                    .with_section(|| {
                        String::from_utf8_lossy(&output.stderr)
                            .to_string()
                            .header("Stderr:")
                    });
            }
            let output = String::from_utf8(output.stdout)?;
            let output = output.trim();
            output
                .try_into()
                .unwrap_or(RustcTargetPlatform::Unsupported(output.to_string()))
        };

        let profile = args.profile().map(Profile::from).unwrap_or(Profile::Debug);
        let target_arch = args.target();

        Ok(Self {
            root,
            build_dir,
            cargo_home,
            profile,
            target_arch,
            host_arch,
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
        self.arch_profile_dir(&unit_info.target_arch)
    }

    pub fn arch_profile_dir(&self, target_arch: &RustcTarget) -> AbsDirPath {
        match target_arch {
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
        // Running `cargo build --build-plan` resets the state in the `target`
        // directory. To work around this we temporarily rename `target`, run
        // the build plan, and move it back. If the rename fails (e.g.,
        // permissions, cross-device), we proceed without it; this will then
        // have the original issue but at least won't break the build.
        let renamed = if fs::exists(&self.build_dir).await {
            debug!("target exists before running build plan, renaming");
            let temp = self
                .root
                .try_join_dir(format!("target.backup.{}", Uuid::new_v4()))?;

            let renamed = fs::rename(&self.build_dir, &temp).await.is_ok();
            debug!(?renamed, ?temp, "renamed temp target");
            if renamed { Some(temp) } else { None }
        } else {
            debug!("target does not exist before running build plan");
            None
        };

        let ret = self.build_plan_inner(args).await;

        if let Some(temp) = renamed {
            debug!("restoring original target");
            fs::remove_dir_all(&self.build_dir).await?;
            fs::rename(&temp, &self.build_dir).await?;
            debug!("restored original target");
        } else {
            // When the build directory didn't exist at the start, we need to
            // clean up the newly created extraneous build directory.
            debug!(build_dir = ?self.build_dir, "build plan done, cleaning up target");
            fs::remove_dir_all(&self.build_dir).await?;
            debug!("build plan done, done cleaning target");
        }

        ret
    }

    #[instrument(name = "Workspace::build_plan_inner")]
    async fn build_plan_inner(
        &self,
        args: impl AsRef<CargoBuildArguments> + std::fmt::Debug,
    ) -> Result<BuildPlan> {
        // TODO: Handle cases where users pass weird options, including if the
        // user themselves passed `--build-plan`.
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
        self.units_from_build_plan(build_plan).await
    }

    /// Parse unit plans from a build plan.
    ///
    /// This is the core parsing logic shared by both `units()` and
    /// cross-compilation variants. It takes a build plan (which may come
    /// from cargo or cross) and parses it into unit structures.
    ///
    /// The build plan must have host-relative paths (for cross builds,
    /// container paths should already be converted before calling this
    /// method).
    #[instrument(name = "Workspace::units_from_build_plan", skip(build_plan))]
    pub(crate) async fn units_from_build_plan(
        &self,
        build_plan: BuildPlan,
    ) -> Result<Vec<UnitPlan>> {
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
                .try_conv::<AbsFilePath>()?
                .relative_to(&self.cargo_home)
                .is_err()
            {
                trace!("skipping: package outside of $CARGO_HOME");
                continue;
            }

            // Figure out what kind of unit this invocation is.
            let package_name = invocation.package_name;
            let package_version = invocation.package_version;
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
                        let src_path = args.src_path().try_into()?;
                        // Sanity check that constructed values match parsed
                        // values.
                        if target_arch != RustcTarget::ImplicitHost {
                            bail!(
                                "build script compilation has specified --target architecture {:?}",
                                target_arch
                            );
                        }
                        // Build scripts are always compiled for the host architecture.
                        let profile_dir = self.arch_profile_dir(&RustcTarget::ImplicitHost);
                        let bsc_unit = BuildScriptCompilationUnitPlan {
                            info: UnitPlanInfo {
                                unit_hash: unit_hash.into(),
                                package_name,
                                package_version,
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
                        let program = invocation.program.try_conv::<AbsFilePath>()?;
                        let out_dir = invocation
                            .env
                            .remove("OUT_DIR")
                            .ok_or_eyre("build script execution should set OUT_DIR")?
                            .try_conv::<AbsDirPath>()?;
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
                                package_version,
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
                let src_path = args.src_path().try_into()?;
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
                        package_version,
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
}

/// This is a newtype for unit hash strings.
#[derive(Debug, Display, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct UnitHash(String);

impl From<UnitHash> for String {
    fn from(value: UnitHash) -> Self {
        value.0
    }
}
impl From<&UnitHash> for String {
    fn from(value: &UnitHash) -> Self {
        value.0.clone()
    }
}

impl From<String> for UnitHash {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&UnitHash> for UnitHash {
    fn from(value: &UnitHash) -> Self {
        value.clone()
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

    /// The package version of this unit.
    ///
    /// This is primarily used for debugging.
    pub package_version: String,

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

    pub async fn touch(&self, ws: &Workspace, mtime: SystemTime) -> Result<()> {
        match self {
            UnitPlan::LibraryCrate(plan) => plan.touch(ws, mtime).await,
            UnitPlan::BuildScriptCompilation(plan) => plan.touch(ws, mtime).await,
            UnitPlan::BuildScriptExecution(plan) => plan.touch(ws, mtime).await,
        }
    }

    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        match self {
            UnitPlan::LibraryCrate(plan) => plan.fingerprint_json_file(),
            UnitPlan::BuildScriptCompilation(plan) => plan.fingerprint_json_file(),
            UnitPlan::BuildScriptExecution(plan) => plan.fingerprint_json_file(),
        }
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        match self {
            UnitPlan::LibraryCrate(plan) => plan.fingerprint_hash_file(),
            UnitPlan::BuildScriptCompilation(plan) => plan.fingerprint_hash_file(),
            UnitPlan::BuildScriptExecution(plan) => plan.fingerprint_hash_file(),
        }
    }

    pub fn src_path(&self) -> Option<AbsFilePath> {
        match self {
            UnitPlan::LibraryCrate(plan) => Some(plan.src_path.clone()),
            UnitPlan::BuildScriptCompilation(plan) => Some(plan.src_path.clone()),
            UnitPlan::BuildScriptExecution(_) => None,
        }
    }

    pub async fn read_fingerprint(&self, ws: &Workspace) -> Result<Fingerprint> {
        match self {
            UnitPlan::LibraryCrate(plan) => plan.read_fingerprint(ws).await,
            UnitPlan::BuildScriptCompilation(plan) => plan.read_fingerprint(ws).await,
            UnitPlan::BuildScriptExecution(plan) => plan.read_fingerprint(ws).await,
        }
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
