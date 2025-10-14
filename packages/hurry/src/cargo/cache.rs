use std::{
    collections::HashMap,
    fmt::Debug,
    io::Write,
    path::PathBuf,
    str::FromStr as _,
    time::{Duration, UNIX_EPOCH},
};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt, bail},
};
use futures::TryStreamExt as _;
use serde::Serialize;
use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tap::Pipe as _;
use tracing::{debug, error, instrument, trace, warn};

use crate::{
    cargo::{
        self, BuildPlan, CargoBuildArguments, CargoCompileMode, Profile, RustcMetadata, Workspace,
    },
    cas::FsCas,
    fs,
    hash::Blake3,
    mk_rel_dir, mk_rel_file,
    path::{AbsDirPath, AbsFilePath, JoinWith as _, TryJoinWith as _},
};

#[derive(Debug, Clone)]
pub struct CargoCache {
    cas: FsCas,
    db: SqlitePool,
    ws: Workspace,
}

impl CargoCache {
    /// The migrator for the database.
    pub const MIGRATOR: Migrator = sqlx::migrate!("./schema/migrations");

    #[instrument(name = "CargoCache::open")]
    async fn open(cas: FsCas, conn: &str, ws: Workspace) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(conn)
            .context("parse sqlite connection string")?
            .create_if_missing(true);
        let db = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .context("connecting to cargo cache database")?;
        Self::MIGRATOR
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
    pub async fn artifact_plan(
        &self,
        profile: &Profile,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<Vec<ArtifactPlan>> {
        let rustc = RustcMetadata::from_argv(&self.ws.root, &args)
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
        //
        // FIXME: Why does running this clear all the compiled artifacts from
        // the target folder?
        let mut build_args = args.as_ref().to_argv();
        build_args.extend([
            String::from("--build-plan"),
            String::from("-Z"),
            String::from("unstable-options"),
        ]);

        let build_plan = cargo::invoke_output("build", build_args, [("RUSTC_BOOTSTRAP", "1")])
            .await?
            .pipe(|output| serde_json::from_slice::<BuildPlan>(&output.stdout))
            .context("parsing build plan")?;
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
                        .ok_or_eyre("no unit hash suffix")?
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
                artifacts.push(ArtifactPlan {
                    package_name: invocation.package_name,
                    package_version: invocation.package_version,
                    // TODO: We assume it's the same target as the host, but we
                    // really should be parsing this from the `rustc`
                    // invocation.
                    target: rustc.host_target.clone(),
                    profile: profile.clone(),
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

        Ok(artifacts)
    }

    #[instrument(name = "CargoCache::save")]
    pub async fn save(&self, artifact: BuiltArtifact) -> Result<()> {
        // TODO: We should probably not be re-locking and unlocking on a per-artifact
        // basis. Maybe this method should instead take a Vec?
        let profile_dir = self.ws.open_profile_locked(&artifact.profile).await?;

        // Determine which files will be saved.
        let lib_files = {
            let lib_fingerprint_dir = profile_dir.root().try_join_dirs(&[
                String::from(".fingerprint"),
                format!(
                    "{}-{}",
                    artifact.package_name, artifact.library_crate_compilation_unit_hash
                ),
            ])?;
            let lib_fingerprint_files = fs::walk_files(&lib_fingerprint_dir)
                .try_collect::<Vec<_>>()
                .await?;
            artifact
                .lib_files
                .into_iter()
                .chain(lib_fingerprint_files)
                .collect::<Vec<_>>()
        };
        let build_script_files = match artifact.build_script_files {
            Some(build_script_files) => {
                let compiled_files = fs::walk_files(&build_script_files.compiled_dir)
                    .try_collect::<Vec<_>>()
                    .await?;
                let compiled_fingerprint_dir = profile_dir.root().try_join_dirs(&[
                    String::from(".fingerprint"),
                    format!(
                        "{}-{}",
                        artifact.package_name,
                        artifact
                            .build_script_compilation_unit_hash
                            .as_ref()
                            .expect("build script files have compilation unit hash")
                    ),
                ])?;
                let compiled_fingerprint_files = fs::walk_files(&compiled_fingerprint_dir)
                    .try_collect::<Vec<_>>()
                    .await?;
                let output_files = fs::walk_files(&build_script_files.output_dir)
                    .try_collect::<Vec<_>>()
                    .await?;
                let output_fingerprint_dir = profile_dir.root().try_join_dirs(&[
                    String::from(".fingerprint"),
                    format!(
                        "{}-{}",
                        artifact.package_name,
                        artifact
                            .build_script_execution_unit_hash
                            .as_ref()
                            .expect("build script files have execution unit hash")
                    ),
                ])?;
                let output_fingerprint_files = fs::walk_files(&output_fingerprint_dir)
                    .try_collect::<Vec<_>>()
                    .await?;
                compiled_files
                    .into_iter()
                    .chain(compiled_fingerprint_files)
                    .chain(output_files)
                    .chain(output_fingerprint_files)
                    .collect()
            }
            None => vec![],
        };
        let files_to_save = lib_files.into_iter().chain(build_script_files);

        // For each file, save it into the CAS and calculate its key.
        //
        // TODO: Fuse this operation with the loop above where we discover the
        // needed files? Would that give better performance?
        let mut library_unit_files = vec![];
        for path in files_to_save {
            match fs::read_buffered(&path).await? {
                Some(content) => {
                    let key = self.cas.store(&content).await?;
                    debug!(?path, ?key, "stored object");
                    library_unit_files.push((path, key));
                }
                None => {
                    // Note that this is not necessarily incorrect! For example,
                    // Cargo seems to claim to emit `.dwp` files for its `.so`s,
                    // but those don't seem to be there by the time the process
                    // actually finishes. I'm not sure if they're deleted or
                    // just never written.
                    warn!("failed to read file: {}", path);
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
        debug!(?content_hash, "calculated content hash");

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
                AND COALESCE(build_script_compilation_unit_hash, '') = COALESCE($4, '')
                AND COALESCE(build_script_execution_unit_hash, '') = COALESCE($5, '')
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
        // See if there are any saved artifacts that match.
        let mut tx = self.db.begin().await?;
        let unit_builds = sqlx::query!(
            r#"
            SELECT
                library_unit_build.id AS "id!: i64",
                library_unit_build.content_hash
            FROM package
            JOIN library_unit_build ON package.id = library_unit_build.package_id
            WHERE
                package.name = $1
                AND package.version = $2
                AND target = $3
                AND library_crate_compilation_unit_hash = $4
                AND COALESCE(build_script_compilation_unit_hash, '') = COALESCE($5, '')
                AND COALESCE(build_script_execution_unit_hash, '') = COALESCE($6, '')
        "#,
            artifact.package_name,
            artifact.package_version,
            artifact.target,
            artifact.library_crate_compilation_unit_hash,
            artifact.build_script_compilation_unit_hash,
            artifact.build_script_execution_unit_hash
        )
        .fetch_all(&mut *tx)
        .await?;

        let unit_to_restore = match unit_builds.split_first() {
            // If there is one matching unit, just restore that one.
            Some((first, [])) => first.id,
            // If there are multiple matching library units, choose the
            // canonical unit to restore.
            //
            // TODO: We only do this today because our keys are
            // insufficiently precise (in particular, we aren't able to
            // key on predicted dynamic fields from build script
            // execution). We can probably do a lot better.
            Some((first, rest)) => {
                if rest
                    .iter()
                    .all(|unit| unit.content_hash == first.content_hash)
                {
                    // If all the units have the same content hash, then we
                    // can restore any of them. This should generally not
                    // happen, but can occur sometimes due to cache database
                    // corruption.
                    first.id
                } else {
                    // If there are any units with different content hash,
                    // then we should emit a warning and choose not to
                    // restore any of them.
                    warn!(
                        ?artifact,
                        ?unit_builds,
                        "multiple matching library unit builds found"
                    );
                    return Ok(());
                }
            }
            // If there are no matching library units, there's nothing to restore.
            None => {
                debug!(?artifact, "no matching library unit build found");
                return Ok(());
            }
        };

        // Restore the unit.
        let objects = sqlx::query!(
            r#"
            SELECT
                object.key,
                library_unit_build_artifact.path,
                library_unit_build_artifact.mtime,
                library_unit_build_artifact.executable
            FROM library_unit_build_artifact
            JOIN object ON library_unit_build_artifact.object_id = object.id
            WHERE
                library_unit_build_artifact.library_unit_build_id = $1
        "#,
            unit_to_restore
        )
        .fetch_all(&mut *tx)
        .await?;
        for object in objects {
            // TODO: Why is this backed by a String? Why don't we store this as
            // a BLOB?
            let key = Blake3::from_hex_string(&object.key)?;
            // TODO: Instead of reading and then writing, maybe we should change
            // the API shape to directly do a copy on supported filesystems?
            let data = self.cas.must_get(&key).await?;
            // TODO: These are currently all absolute paths. We need to
            // implement relative path rewrites for portability.
            let path = AbsFilePath::try_from(object.path)?;
            let mtime = {
                let mtime_bytes: Result<[u8; 16], _> = object.mtime.try_into();
                let mtime_nanos =
                    u128::from_be_bytes(mtime_bytes.or_else(|_| bail!("could not read mtime"))?);
                // TODO: Is this conversion safe? It will truncate, but probably
                // not outside the range we care about. Maybe we really should
                // serialize to TEXT just to get rid of this headache.
                UNIX_EPOCH + Duration::from_nanos(mtime_nanos as u64)
            };
            let metadata = fs::Metadata {
                mtime,
                executable: object.executable,
            };
            fs::write(&path, &data).await?;
            metadata.set_file(&path).await?;
        }
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
    profile: Profile,

    // Artifact folders to save and restore.
    lib_files: Vec<AbsFilePath>,
    build_script_files: Option<BuildScriptDirs>,

    // Unit hashes.
    library_crate_compilation_unit_hash: String,
    build_script_compilation_unit_hash: Option<String>,
    build_script_execution_unit_hash: Option<String>,
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
    profile: Profile,

    lib_files: Vec<AbsFilePath>,
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
        Ok(BuiltArtifact {
            package_name: plan.package_name,
            package_version: plan.package_version,
            target: plan.target,
            profile: plan.profile,

            lib_files: plan.lib_files,
            build_script_files: plan.build_script_files,

            library_crate_compilation_unit_hash: plan.library_crate_compilation_unit_hash,
            build_script_compilation_unit_hash: plan.build_script_compilation_unit_hash,
            build_script_execution_unit_hash: plan.build_script_execution_unit_hash,
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq as pretty_assert_eq;

    #[sqlx::test(migrator = "crate::cargo::cache::CargoCache::MIGRATOR")]
    async fn open_test_database(pool: SqlitePool) {
        sqlx::query("select 1")
            .fetch_one(&pool)
            .await
            .expect("select 1");
    }

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
