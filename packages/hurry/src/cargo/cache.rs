use std::{collections::HashMap, io::Write, path::PathBuf, str::FromStr as _, time::UNIX_EPOCH};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt, bail},
};
use futures::StreamExt;
use serde::Serialize;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tap::Pipe as _;
use tracing::{debug, error, instrument, trace, warn};

use crate::{
    cargo::{self, BuildPlan, CargoCompileMode, Profile, RustcMetadata, Workspace},
    cas::FsCas,
    fs,
    hash::Blake3,
    mk_rel_dir, mk_rel_file,
    path::{AbsDirPath, AbsFilePath, JoinWith as _},
};

#[derive(Debug, Clone)]
pub struct CargoCache {
    cas: FsCas,
    db: SqlitePool,
    ws: Workspace,
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

        // TODO: Pass the rest of the `cargo build` flags in, so the build plan
        // is an accurate reflection of the user's build.
        //
        // FIXME: Why does running this clear all the compiled artifacts from
        // the target folder?
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
        for (i, invocation) in build_plan.invocations.clone().into_iter().enumerate() {
            trace!(?invocation, "build plan invocation");
            // For each invocation, figure out what kind it is:
            // 1. Compiling a build script.
            // 2. Running a build script.
            // 3. Compiling a dependency.
            // 4. Compiling first-party code.
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
                    if dep.target_kind == [TargetKind::CustomBuild]
                        && dep.compile_mode == CargoCompileMode::RunCustomBuild
                        && dep.package_name == invocation.package_name
                        && dep.package_version == invocation.package_version
                    {
                        build_script_execution_index = Some(dep_index);
                        break;
                    }
                }

                let compiled_files: Vec<AbsFilePath> = invocation
                    .outputs
                    .into_iter()
                    .map(|f| AbsFilePath::try_from(f).unwrap())
                    .collect();
                let build_script = match build_script_execution_index {
                    Some(build_script_execution_index) => {
                        let (build_script_index, build_script_output_dir) = build_script_executions
                            .get(build_script_execution_index)
                            .ok_or_eyre(
                                "build script execution should have recorded output directory",
                            )?;
                        let build_script_output_dir =
                            AbsDirPath::try_from(build_script_output_dir)?;
                        let build_script_compiled_dir = build_script_index_to_dir
                            .get(build_script_index)
                            .ok_or_eyre(
                                "build script index should have recorded compilation directory",
                            )?;
                        let build_script_compiled_dir =
                            AbsDirPath::try_from(build_script_compiled_dir)?;
                        Some(BuildScriptDirs {
                            compiled_dir: build_script_compiled_dir,
                            output_dir: build_script_output_dir,
                        })
                    }
                    None => None,
                };

                // Given a dependency being compiled, we need to determine the
                // compiled files, its build script directory, and its build
                // script outputs directory. These are the files that we're
                // going to save for this artifact.
                debug!(
                    compiled = ?compiled_files,
                    build_script = ?build_script,
                    deps = ?invocation.deps,
                    "artifacts to save"
                );
                artifacts.push(ArtifactPlan {
                    package_name: invocation.package_name,
                    package_version: invocation.package_version,
                    // TODO: We assume it's the same target as the host, but we
                    // really should be parsing this from the `rustc`
                    // invocation.
                    target: rustc.host_target.clone(),
                    compiled_files,
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

    #[instrument(name = "CargoCache::save")]
    pub async fn save(&self, artifact: BuiltArtifact) -> Result<()> {
        // Determine which files will be saved.
        let compiled_files = artifact.compiled_files;
        let build_script_files = match artifact.build_script_files {
            Some(build_script_files) => {
                let compiled_files = fs::walk_files(&build_script_files.compiled_dir)
                    .collect::<Vec<_>>()
                    .await
                    .into_iter()
                    .collect::<Result<Vec<_>>>()?;
                let output_files = fs::walk_files(&build_script_files.output_dir)
                    .collect::<Vec<_>>()
                    .await
                    .into_iter()
                    .collect::<Result<Vec<_>>>()?;
                compiled_files
                    .into_iter()
                    .chain(output_files.into_iter())
                    .collect()
            }
            None => vec![],
        };
        let files_to_save = compiled_files
            .into_iter()
            .chain(build_script_files.into_iter())
            .collect::<Vec<_>>();

        // For each file, save it into the CAS and calculate its key.
        //
        // TODO: Fuse this operation with the loop above where we discover the
        // needed files? Would that give better performance?
        let mut library_unit_files = vec![];
        for file in files_to_save {
            match fs::read_buffered(&file).await? {
                Some(content) => {
                    let key = self.cas.store(&content).await?;
                    library_unit_files.push((file, key));
                }
                None => {
                    // Note that this is not necessarily incorrect! For example,
                    // Cargo seems to claim to emit `.dwp` files for its `.so`s,
                    // but those don't seem to be there by the time the process
                    // actually finishes. I'm not sure if they're deleted or
                    // just never written.
                    warn!("failed to read file: {}", file);
                }
            }
        }

        // Calculate the content hash.
        let content_hash = {
            let mut hasher = blake3::Hasher::new();
            let bytes = serde_json::to_vec(&LibraryUnitHash::new(library_unit_files.clone()))?;
            hasher.write_all(&bytes)?;
            hasher.finalize().to_hex().to_string()
        };

        // Save the library unit into the database.
        let mut tx = self.db.begin().await?;

        // Find or create the package.
        let package_id = match sqlx::query!(
            // TODO: Why does this require a type override? Shouldn't sqlx infer
            // the non-nullability from the INTEGER PRIMARY KEY column type?
            "SELECT id AS \"id!: i64\" FROM package WHERE name = $1 AND version = $2",
            artifact.package_name,
            artifact.package_version
        )
        .fetch_optional(&mut *tx)
        .await?
        {
            Some(row) => row.id,
            None => {
                sqlx::query!(
                    "INSERT INTO package (name, version) VALUES ($1, $2) RETURNING id",
                    artifact.package_name,
                    artifact.package_version
                )
                .fetch_one(&mut *tx)
                .await?
                .id
            }
        };
        // Check whether a library unit build exists.
        match sqlx::query!(
            r#"
            SELECT content_hash
            FROM library_unit_build
            WHERE
                package_id = $1
                AND target = $2
                AND library_crate_compilation_unit_hash = $3
                AND build_script_compilation_unit_hash = $4
                AND build_script_execution_unit_hash = $5
            "#,
            package_id,
            artifact.target,
            artifact.library_crate_compilation_unit_hash,
            artifact.build_script_compilation_unit_hash,
            artifact.build_script_execution_unit_hash
        )
        .fetch_optional(&mut *tx)
        .await?
        {
            Some(row) => {
                // If it does exist, and the content hash is the same, there is
                // nothing more to do. If it exists but the content hash is
                // different, then something has gone wrong with our cache key,
                // and we should log an error message.
                if row.content_hash != content_hash {
                    error!(expected = ?row.content_hash, actual = ?content_hash, "content hash mismatch");
                }
            }
            None => {
                // Insert the library unit build.
                let library_unit_build_id = sqlx::query!(
                    r#"
                    INSERT INTO library_unit_build (
                        package_id,
                        target,
                        library_crate_compilation_unit_hash,
                        build_script_compilation_unit_hash,
                        build_script_execution_unit_hash,
                        content_hash
                    ) VALUES ($1, $2, $3, $4, $5, $6)
                    RETURNING id AS "id!: i64"
                    "#,
                    package_id,
                    artifact.target,
                    artifact.library_crate_compilation_unit_hash,
                    artifact.build_script_compilation_unit_hash,
                    artifact.build_script_execution_unit_hash,
                    content_hash
                )
                .fetch_one(&mut *tx)
                .await?
                .id;

                // Insert each file.
                for (file, key) in library_unit_files {
                    let key = key.as_str();
                    // Find or create CAS object.
                    let object_id = match sqlx::query!(
                        "SELECT id AS \"id!: i64\" FROM object WHERE key = $1",
                        key
                    )
                    .fetch_optional(&mut *tx)
                    .await?
                    {
                        Some(row) => row.id,
                        None => {
                            sqlx::query!("INSERT INTO object (key) VALUES ($1) RETURNING id", key)
                                .fetch_one(&mut *tx)
                                .await?
                                .id
                        }
                    };

                    // TODO: Would it be faster to gather this during the
                    // walking?
                    let metadata = fs::Metadata::from_file(&file)
                        .await?
                        .ok_or_eyre("could not stat file metadata")?;

                    // We need to do this because SQLite does not support
                    // 128-bit integers.
                    let mtime_bytes = metadata
                        .mtime
                        .duration_since(UNIX_EPOCH)?
                        .as_nanos()
                        .to_be_bytes();
                    let mtime_slice = mtime_bytes.as_slice();

                    let filepath = file.to_string();

                    sqlx::query!(
                        r#"
                        INSERT INTO library_unit_build_artifact (
                            library_unit_build_id,
                            object_id,
                            path,
                            mtime,
                            executable
                        ) VALUES ($1, $2, $3, $4, $5)
                         "#,
                        library_unit_build_id,
                        object_id,
                        filepath,
                        mtime_slice,
                        metadata.executable
                    )
                    .execute(&mut *tx)
                    .await?;
                }
            }
        };

        tx.commit().await?;

        Ok(())
    }

    #[instrument(name = "CargoCache::restore")]
    pub async fn restore(&self, artifact: &ArtifactPlan) -> Result<()> {
        // TODO: Implement.
        //
        // TODO: Make sure to warn on ambiguous restores.
        Ok(())
    }
}

/// An ArtifactPlan represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) statically
/// at plan-time.
///
/// In particular, this information does _not_ include information derived from
/// compiling and running the build script, such as `rustc` flags from build
/// script output directives.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArtifactPlan {
    // Partial artifact key information. Note that this is only derived from the
    // build plan, and therefore is missing essential information (e.g. `rustc`
    // flags from build script output directives) that can only be determined
    // interactively.
    //
    // TODO: There are more fields here that we can know from the planning stage
    // that need to be added (e.g. features).
    package_name: String,
    package_version: String,
    target: String,

    // Artifact folders to save and restore.
    compiled_files: Vec<AbsFilePath>,
    build_script_files: Option<BuildScriptDirs>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct BuildScriptDirs {
    compiled_dir: AbsDirPath,
    output_dir: AbsDirPath,
}

/// A BuiltArtifact represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) after it
/// has been built.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct BuiltArtifact {
    package_name: String,
    package_version: String,

    target: String,

    compiled_files: Vec<AbsFilePath>,
    build_script_files: Option<BuildScriptDirs>,

    library_crate_compilation_unit_hash: String,
    build_script_compilation_unit_hash: Option<String>,
    build_script_execution_unit_hash: Option<String>,
}

impl BuiltArtifact {
    /// Given an `ArtifactPlan`, read the build script output directories on
    /// disk and construct a `BuiltArtifact`.
    #[instrument(name = "BuiltArtifact::from_plan")]
    pub async fn from_plan(plan: ArtifactPlan) -> Result<Self> {
        // TODO: Read the build script output from the build folders, and parse
        // the output for directives. Use this to construct the rustc
        // invocation, and use all of this information to fully construct the
        // cache key.

        // FIXME: What we actually do right now is just copy fields and ignore
        // that dynamic fields might not be captured by the unit hash. This
        // behavior is incorrect! We are only ignoring this for now so we can
        // get something simple working end-to-end.

        let library_crate_compilation_unit_hash = {
            let compiled_file = plan
                .compiled_files
                .first()
                .ok_or_eyre("no compiled files")?;
            let filename = compiled_file
                .file_name()
                .ok_or_eyre("no filename")?
                .to_string_lossy();
            let filename = filename.split_once('.').ok_or_eyre("no extension")?.0;

            filename
                .rsplit_once('-')
                .ok_or_eyre("no unit hash suffix")?
                .1
                .to_string()
        };
        let (build_script_compilation_unit_hash, build_script_execution_unit_hash) =
            match &plan.build_script_files {
                Some(build_script_files) => {
                    let build_script_compilation_unit_hash = {
                        let filename = &build_script_files
                            .compiled_dir
                            .file_name()
                            .ok_or_eyre("no filename")?
                            .to_string_lossy();

                        filename
                            .rsplit_once('-')
                            .ok_or_eyre("no unit hash suffix")?
                            .1
                            .to_string()
                    };
                    let build_script_execution_unit_hash = {
                        let out_dir_path = &build_script_files
                            .output_dir
                            .parent()
                            .ok_or_eyre("out_dir has no parent")?;
                        let filename = out_dir_path
                            .file_name()
                            .ok_or_eyre("out_dir has no filename")?
                            .to_string_lossy();

                        filename
                            .rsplit_once('-')
                            .ok_or_eyre("no unit hash suffix")?
                            .1
                            .to_string()
                    };
                    (
                        Some(build_script_compilation_unit_hash),
                        Some(build_script_execution_unit_hash),
                    )
                }
                None => (None, None),
            };

        Ok(BuiltArtifact {
            package_name: plan.package_name,
            package_version: plan.package_version,

            target: plan.target,

            compiled_files: plan.compiled_files,
            build_script_files: plan.build_script_files,

            library_crate_compilation_unit_hash,
            build_script_compilation_unit_hash,
            build_script_execution_unit_hash,
        })
    }
}

/// A content hash of a library unit's artifacts.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize)]
struct LibraryUnitHash {
    files: Vec<(AbsFilePath, Blake3)>,
}

impl LibraryUnitHash {
    /// Construct a library unit hash out of the files in the library unit.
    ///
    /// This constructor always ensures that the files are sorted, so any two
    /// sets of files with the same paths and contents will produce the same
    /// hash.
    fn new(mut files: Vec<(AbsFilePath, Blake3)>) -> Self {
        files.sort();
        Self { files }
    }
}
