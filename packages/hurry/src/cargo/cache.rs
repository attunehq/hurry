use std::{collections::HashMap, path::PathBuf, str::FromStr as _};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt, bail},
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tap::Pipe as _;
use tracing::{debug, instrument, trace};

use crate::{
    Locked,
    cargo::{
        self, BuildPlan, CargoCompileMode, Profile, ProfileDir, RustcMetadata, UnitGraph, Workspace,
    },
    cas::FsCas,
    fs, mk_rel_dir, mk_rel_file,
    path::{AbsDirPath, JoinWith as _},
};

#[derive(Debug, Clone)]
pub struct CargoCache {
    cas: FsCas,
    db: SqlitePool,
    pub ws: Workspace,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open")]
    async fn open(cas: FsCas, conn: &str, ws: Workspace) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(conn)
            .context("parse sqlite connection string")?
            .create_if_missing(true);
        let db = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .context("connecting to cargo cache database")?;
        sqlx::migrate!("src/cargo/cache/db/migrations")
            .run(&db)
            .await
            .context("running migrations")?;
        Ok(Self { cas, db, ws })
    }

    #[instrument(name = "CargoCache::open_dir")]
    pub async fn open_dir(cas: FsCas, cache_dir: &AbsDirPath, ws: Workspace) -> Result<Self> {
        let dbfile = cache_dir.join(mk_rel_file!("cache.db"));
        fs::create_dir_all(cache_dir)
            .await
            .context("create cache directory")?;

        Self::open(cas, &format!("sqlite://{}", dbfile), ws).await
    }

    #[instrument(name = "CargoCache::open_default")]
    pub async fn open_default(ws: Workspace) -> Result<Self> {
        let cas = FsCas::open_default().await.context("opening CAS")?;
        let cache = fs::user_global_cache_path()
            .await
            .context("finding user cache path")?
            .join(mk_rel_dir!("cargo"));
        Self::open_dir(cas, &cache, ws).await
    }

    #[instrument(name = "CargoCache::artifacts")]
    pub async fn artifact_plan(&self, profile: &Profile) -> Result<Vec<ArtifactPlan>> {
        let rustc = RustcMetadata::from_argv(&self.ws.root, &[])
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

        // TODO: Pass the rest of the `cargo build` flags in.
        let build_plan = cargo::invoke_output(
            "build",
            ["--build-plan", "-Z", "unstable-options"],
            [("RUSTC_BOOTSTRAP", "1")],
        )
        .await?
        .pipe(|output| serde_json::from_slice::<BuildPlan>(&output.stdout))
        .context("parsing build plan")?;
        trace!(?build_plan, "build plan");

        let mut build_script_index_to_dir = HashMap::new();
        let mut build_script_program_file_to_index = HashMap::new();
        let mut build_script_executions = HashMap::new();
        let mut artifacts = Vec::new();
        for (i, invocation) in build_plan.invocations.iter().enumerate() {
            trace!(?invocation, "build plan invocation");
            // For each invocation, figure out what kind it is:
            // 1. Compiling a build script.
            // 2. Running a build script.
            // 3. Compiling a dependency.
            // 4. Compiling first-party code.
            if invocation.target_kind == &[TargetKind::CustomBuild] {
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
                            for file in &invocation.outputs {
                                build_script_program_file_to_index.insert(file, i);
                            }
                            for (fslink, _orig) in &invocation.links {
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
                            .ok_or_eyre("build script execution should set OUT_DIR")?;

                        build_script_executions.insert(i, (build_script_index, out_dir));
                    }
                    _ => bail!(
                        "unknown compile mode for build script: {:?}",
                        invocation.compile_mode
                    ),
                }
            } else if invocation.target_kind == &[TargetKind::Bin] {
                // Binaries are _always_ first-party code. Do nothing for now.
                continue;
            } else if invocation.target_kind.contains(&TargetKind::Lib)
                || invocation.target_kind.contains(&TargetKind::RLib)
                || invocation.target_kind.contains(&TargetKind::CDyLib)
                || invocation.target_kind.contains(&TargetKind::ProcMacro)
            {
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
                    // This should be sufficient to deermine which dependency is
                    // the execution of the build script of the current library.
                    // There might be other build scripts for the same name and
                    // version (but different features), but they won't be
                    // listed as a `dep`.
                    if dep.target_kind == &[TargetKind::CustomBuild]
                        && dep.compile_mode == CargoCompileMode::RunCustomBuild
                        && dep.package_name == invocation.package_name
                        && dep.package_version == invocation.package_version
                    {
                        build_script_execution_index = Some(dep_index);
                        break;
                    }
                }

                let build_script = match build_script_execution_index {
                    Some(build_script_execution_index) => {
                        let (build_script_index, build_script_output_dir) = build_script_executions
                            .get(&build_script_execution_index)
                            .ok_or_eyre(
                                "build script execution should have recorded output directory",
                            )?;
                        let build_script_dir = build_script_index_to_dir
                            .get(build_script_index)
                            .ok_or_eyre(
                                "build script index should have recorded compilation directory",
                            )?;
                        Some(BuildScriptDirs {
                            compiled_dir: build_script_dir.to_string_lossy().to_string(),
                            output_dir: build_script_output_dir.to_string(),
                        })
                    }
                    None => None,
                };

                // Given a dependency being compiled, we need to determine the
                // compiled files, its build script directory, and its build
                // script outputs directory. These are the files that we're
                // going to save for this artifact.
                debug!(
                    compiled = ?invocation.outputs,
                    build_script = ?build_script,
                    deps = ?invocation.deps,
                    "artifacts to save"
                );
                artifacts.push(ArtifactPlan {
                    package_name: invocation.package_name.clone(),
                    package_version: invocation.package_version.clone(),
                    compiled_files: invocation.outputs.clone(),
                    build_script_files: build_script,
                });

                // TODO: If needed, we could try to read previous build script
                // output from the target directory here to try and supplement
                // information for built crates. I can't imagine why we would
                // need to do that, though.
            } else {
                bail!("unknown target kind: {:?}", invocation.target_kind);
            }
        }

        Ok(artifacts)
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArtifactPlan {
    // Partial artifact key information. Note that this is only derived from the
    // build plan, and therefore is missing essential information (e.g. `rustc`
    // flags from build script output directives) that can only be determined
    // interactively.
    //
    // TODO: There are more fields here that we can know from the planning stage
    // that need to be added (e.g. target, features).
    package_name: String,
    package_version: String,

    // Artifact folders to save and restore.
    //
    // TODO: These should probably be `QualifiedPath`s.
    compiled_files: Vec<String>,
    build_script_files: Option<BuildScriptDirs>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct BuildScriptDirs {
    // TODO: These should probably be `QualifiedPath`s.
    compiled_dir: String,
    output_dir: String,
}

/*


    /// Store the contents of the file referenced by the path in the CAS.
    #[instrument(name = "ProfileDir::store_cas")]
    pub async fn store_cas(
        &self,
        cas: &FsCas,
        file: &RelFilePath,
    ) -> Result<(Blake3, AbsFilePath)> {
        let file = self.root.join(file);
        let raw = fs::must_read_buffered(&file).await.context("read file")?;

        // TODO: If we keep rewrites with this sort of structure we probably
        // want to turn them into a more generic operation instead of having to
        // retype all this boilerplate every time.
        let content = match CasRewrite::from_path(&file) {
            CasRewrite::None => raw,
            CasRewrite::RootOutput => RootOutput::from_file(self, &file)
                .await
                .context("parse")?
                .pipe_ref(serde_json::to_vec)
                .context("serialize")?,
            CasRewrite::BuildScriptOutput => BuildScriptOutput::from_file(self, &file)
                .await
                .context("parse")?
                .pipe_ref(serde_json::to_vec)
                .context("serialize")?,
            CasRewrite::DepInfo => DepInfo::from_file(self, &file)
                .await
                .context("parse")?
                .pipe_ref(serde_json::to_vec)
                .context("serialize")?,
        };

        cas.store(&content)
            .await
            .context("store content in CAS")
            .map(|key| (key, file))
    }

    /// Get the content from the CAS referenced by the key and restore it
    /// to the provided path.
    #[instrument(name = "ProfileDir::restore_cas")]
    pub async fn restore_cas(
        &self,
        cas: &FsCas,
        key: &Blake3,
        file: &RelFilePath,
    ) -> Result<AbsFilePath> {
        let file = self.root.join(file);
        let content = cas.must_get(key).await.context("get content from CAS")?;

        // TODO: If we keep rewrites with this sort of structure we probably
        // want to turn them into a more generic operation instead of having to
        // retype all this boilerplate every time.
        let raw = match CasRewrite::from_path(&file) {
            CasRewrite::None => content,
            CasRewrite::RootOutput => serde_json::from_slice::<RootOutput>(&content)
                .context("deserialize")
                .map(|f| f.reconstruct(self).into_bytes())?,
            CasRewrite::BuildScriptOutput => serde_json::from_slice::<BuildScriptOutput>(&content)
                .context("deserialize")
                .map(|f| f.reconstruct(self).into_bytes())?,
            CasRewrite::DepInfo => serde_json::from_slice::<DepInfo>(&content)
                .context("deserialize")
                .map(|f| f.reconstruct(self).into_bytes())?,
        };
        fs::write(&file, &raw)
            .await
            .context("write file")
            .map(|_| file)
    }
*/

/*


/// Some files need to be rewritten when stored in or restored from the CAS.
/// This type supports parsing a path to determine whether it should be
/// rewritten, and if so using what strategy.
///
/// A core intention of this type is to _always_ only replace things that
/// `hurry` can actually _parse_- no blanket "replace all" functionality.
/// The reasoning here is that while we want to make builds faster,
/// we **cannot** make them incorrect; if we skip rewriting something
/// that Cargo needs it'll simply recompile while if we overzealously rewrite
/// things we don't actually know anything about we might cause subtle and
/// bugs in the compilation phase which we absolutely cannot afford to do.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default)]
enum CasRewrite {
    /// No rewriting should take place.
    #[default]
    None,

    /// This is a "dep-info" file, so use the "dep-info" rewrite strategy.
    DepInfo,

    /// This is a "root output" file, used for build scripts.
    ///
    /// This file contains the fully qualified path to `out`, which is the
    /// directory where script can output files (provided to the script as
    /// $OUT_DIR).
    ///
    /// These are correct to rewrite because the content of the `out` directory
    /// should have also been restored, but even if it wasn't it's certainly not
    /// correct to try to read or write content from the old location.
    ///
    /// Example taken from an actual project:
    /// ```not_rust
    /// /Users/jess/scratch/example/target/debug/build/rustls-5590c033895e7e9a/out
    /// ```
    RootOutput,

    /// This is an "output" file, which is the output of the build script when
    /// it was executed.
    ///
    /// These are correct to rewrite because paths in this output will almost
    /// definitely be referencing either something local or something in
    /// `$CARGO_HOME`.
    ///
    /// Example output taken from an actual project:
    /// ```not_rust
    /// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
    /// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
    /// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
    /// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
    /// cargo:rustc-link-search=native=/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out
    /// cargo:root=/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out
    /// cargo:include=/Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/zstd-sys-2.0.15+zstd.1.5.7/zstd/lib
    /// ```
    ///
    /// Reference: https://doc.rust-lang.org/cargo/reference/build-scripts.html
    BuildScriptOutput,
}

impl CasRewrite {
    /// Determine the rewrite strategy from the file path.
    fn from_path(target: &AbsFilePath) -> Self {
        // `.rev()` and `.take(1)` are because while we're iterating over
        // components, we really don't care about reading the whole path- just
        // the few elements at the end.
        target
            .component_strs_lossy()
            .rev()
            .tuple_windows()
            .take(1)
            .find_map(|(name, _, gparent)| {
                // Theoretically, we could blanket rewrite all paths in all text
                // files- but we follow a conservative approach here because
                // above all we don't want to silently cause miscompilations and
                // we don't want to do more work than is needed.
                //
                // For example, we don't rewrite `stderr` output files for build
                // scripts, because they're only for humans to read. Also some
                // example projects emit other arbitrary text files; e.g. the
                // build script for `aws-lc-sys` emits a file at
                // `./target/debug/build/aws-lc-sys-3f4f475625566422/out/memcmp_invalid_stripped_check.dSYM/Contents/Resources/Relocations/aarch64/memcmp_invalid_stripped_check.yml`
                // which we don't try to replace because we don't really know
                // anything about this file.
                //
                // We do know however that it's common practice in the Rust
                // community to back up and restore files in `target` for
                // caching, so we can only assume that library authors know this
                // and can recover from or at least detect this sort of scenario
                // if they care.
                let ext = name.rsplit_once('.').map(|(_, ext)| ext);
                match (gparent.as_ref(), name.as_ref(), ext) {
                    ("build", "output", _) => Some(Self::BuildScriptOutput),
                    ("build", "root-output", _) => Some(Self::RootOutput),
                    (_, _, Some("d")) => Some(Self::DepInfo),
                    _ => None,
                }
            })
            .unwrap_or_default()
    }
}

*/
