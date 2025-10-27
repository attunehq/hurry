use std::{
    collections::HashMap,
    fmt::Debug,
    io::Write,
    path::PathBuf,
    time::{Duration, UNIX_EPOCH},
};

use crate::progress::TransferBar;
use cargo_metadata::TargetKind;
use clients::{
    Courier,
    courier::v1::{
        Key,
        cache::{ArtifactFile, CargoBulkRestoreHit, CargoRestoreRequest, CargoSaveRequest},
    },
};
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt, bail, eyre},
};
use dashmap::DashSet;
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use rayon::prelude::*;
use scopeguard::defer;
use serde::Serialize;
use tap::Pipe as _;
use tokio::task::JoinSet;
use tracing::{debug, instrument, trace, warn};
use uuid::Uuid;

use crate::{
    cargo::{
        self, BuildPlan, BuildScriptOutput, CargoBuildArguments, CargoCompileMode, DepInfo,
        Profile, QualifiedPath, RootOutput, RustcMetadata, Workspace,
    },
    cas::CourierCas,
    fs, mk_rel_file,
    path::{AbsDirPath, AbsFilePath, JoinWith, TryJoinWith as _},
};

/// Statistics about cache operations.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub files: u64,
    pub bytes: u64,
}

/// Tracks items that were restored from the cache.
#[derive(Debug, Clone, Default)]
pub struct RestoreState {
    pub artifacts: DashSet<ArtifactKey>,
    pub objects: DashSet<Key>,
    pub stats: CacheStats,
}

impl RestoreState {
    /// Records that an artifact was restored from cache.
    fn record_artifact(&self, artifact: &ArtifactKey) {
        self.artifacts.insert(artifact.clone());
    }

    /// Records that an object was restored from cache.
    fn record_object(&self, key: &Key) {
        self.objects.insert(key.clone());
    }

    /// Checks if an artifact was restored from cache.
    fn check_artifact(&self, artifact: &ArtifactKey) -> bool {
        self.artifacts.contains(artifact)
    }

    /// Checks if an object was restored from cache.
    fn check_object(&self, key: &Key) -> bool {
        self.objects.contains(key)
    }

    fn with_stats(self, stats: CacheStats) -> Self {
        Self {
            artifacts: self.artifacts,
            objects: self.objects,
            stats,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CargoCache {
    courier: Courier,
    cas: CourierCas,
    ws: Workspace,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open")]
    pub async fn open(courier: Courier, ws: Workspace) -> Result<Self> {
        let cas = CourierCas::new(courier.clone());
        Ok(Self { cas, courier, ws })
    }

    /// Get the build plan by running `cargo build --build-plan` with the
    /// provided arguments.
    async fn build_plan(&self, args: impl AsRef<CargoBuildArguments> + Debug) -> Result<BuildPlan> {
        // Running `cargo build --build-plan` deletes a bunch of items in the `target`
        // directory. To work around this we temporarily move `target` -> run
        // the build plan -> move it back. If the rename fails (e.g., permissions,
        // cross-device), we proceed without it; this will then have the original issue
        // but at least won't break the build.
        let temp = self
            .ws
            .root
            .as_std_path()
            .join(format!("target.backup.{}", Uuid::new_v4()));

        let renamed = tokio::fs::rename(self.ws.target.as_std_path(), &temp)
            .await
            .is_ok();

        defer! {
            if renamed {
                let target = self.ws.target.as_std_path();
                let _ = std::fs::remove_dir_all(target);
                let _ = std::fs::rename(&temp, target);
            }
        }

        let mut build_args = args.as_ref().to_argv();
        build_args.extend([
            String::from("--build-plan"),
            String::from("-Z"),
            String::from("unstable-options"),
        ]);
        cargo::invoke_output("build", build_args, [("RUSTC_BOOTSTRAP", "1")])
            .await?
            .pipe(|output| serde_json::from_slice::<BuildPlan>(&output.stdout))
            .context("parse build plan")
    }

    #[instrument(name = "CargoCache::artifacts")]
    pub async fn artifact_plan(
        &self,
        profile: &Profile,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<ArtifactPlan> {
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

        Ok(ArtifactPlan {
            artifacts,
            // TODO: We assume it's the same target as the host, but we really
            // should be parsing this from the `rustc` invocation.
            //
            // TODO: Is it possible for different artifacts in the same build to
            // have different targets?
            target: rustc.host_target.clone(),
            profile: profile.clone(),
        })
    }

    #[instrument(name = "CargoCache::save", skip(artifact_plan, progress))]
    pub async fn save(
        &self,
        artifact_plan: ArtifactPlan,
        progress: &TransferBar,
        restored: &RestoreState,
    ) -> Result<CacheStats> {
        trace!(?artifact_plan, "artifact plan");
        for artifact in artifact_plan.artifacts {
            let artifact = BuiltArtifact::from_key(&self.ws, artifact).await?;
            debug!(?artifact, "caching artifact");

            let artifact_key = artifact.reconstruct_key();
            if restored.check_artifact(&artifact_key) {
                trace!(
                    ?artifact_key,
                    "skipping backup: artifact was restored from cache"
                );
                progress.dec_length(1);
                continue;
            }

            let lib_files = self.collect_library_files(&artifact).await?;
            let build_script_files = self.collect_build_script_files(&artifact).await?;
            let files_to_save = lib_files.into_iter().chain(build_script_files).collect();
            let (library_unit_files, artifact_files, bulk_entries) = self
                .process_files_for_upload(files_to_save, restored)
                .await?;

            let uploaded_bytes = self.upload_files_bulk(bulk_entries, progress).await?;
            progress.add_bytes(uploaded_bytes);

            let content_hash = calculate_content_hash(library_unit_files)?;
            debug!(?content_hash, "calculated content hash");

            let request = build_save_request(
                &artifact,
                &artifact_plan.target,
                content_hash,
                artifact_files,
            );

            self.courier.cargo_cache_save(request).await?;
            progress.inc(1);
        }

        Ok(CacheStats {
            files: progress.files(),
            bytes: progress.bytes(),
        })
    }

    #[instrument(name = "CargoCache::restore", skip(artifact_plan, progress))]
    pub async fn restore(
        &self,
        artifact_plan: &ArtifactPlan,
        progress: &TransferBar,
    ) -> Result<RestoreState> {
        trace!(?artifact_plan, "artifact plan");
        let (artifacts, requests) = build_restore_requests(artifact_plan);
        let restore_result = self
            .courier
            .cargo_cache_restore_bulk(requests)
            .await
            .context("cache restore")?;

        for miss in restore_result.misses {
            debug!(artifact = ?miss, "no matching library unit build found");
            progress.dec_length(1);
        }
        let files_to_restore = self
            .filter_files_need_restored(restore_result.hits, &artifacts)
            .await?;

        let restored = RestoreState::default();
        let worker_count = num_cpus::get();
        let (tx, rx) = flume::bounded::<(ArtifactFile, AbsFilePath)>(0);
        let mut workers = self.spawn_restore_workers(worker_count, rx.clone(), progress, &restored);
        for (artifact, files) in files_to_restore {
            for (file, path) in files {
                if let Err(error) = tx.send_async((file, path)).await {
                    panic!("invariant violated: no restore workers are alive: {error:?}");
                }
            }
            restored.record_artifact(&artifact);
            progress.inc(1);
        }

        drop(rx);
        drop(tx);
        while let Some(worker) = workers.join_next().await {
            worker.context("cas restore worker")?;
        }

        Ok(restored.with_stats(CacheStats {
            files: progress.files(),
            bytes: progress.bytes(),
        }))
    }

    /// Filter the set to only the files which need to be restored, either
    /// because they don't exist locally or their hashes don't match.
    async fn filter_files_need_restored(
        &self,
        hits: Vec<CargoBulkRestoreHit>,
        artifacts: &HashMap<Vec<u8>, &ArtifactKey>,
    ) -> Result<HashMap<ArtifactKey, Vec<(ArtifactFile, AbsFilePath)>>> {
        let ws_profile_dir = self.ws.profile_dir.clone();
        let ws_cargo_home = self.ws.cargo_home.clone();
        let artifacts = artifacts
            .iter()
            .map(|(k, &v)| (k.clone(), v.clone()))
            .collect::<HashMap<_, _>>();

        tokio::task::spawn_blocking(move || {
            hits.into_iter()
                .flat_map(|hit| {
                    let request_hash = hit.request.hash();
                    hit.artifacts
                        .into_iter()
                        .map(move |file| (request_hash.clone(), file))
                })
                .par_bridge()
                .filter_map(|(request_hash, file)| {
                    let artifact = artifacts.get(&request_hash)?;
                    let path = serde_json::from_str::<QualifiedPath>(&file.path)
                        .ok()?
                        .reconstruct_raw(&ws_profile_dir, &ws_cargo_home)
                        .pipe(AbsFilePath::try_from)
                        .ok()?;

                    // We use `metadata` instead of `exists` so that we validate permissions too.
                    if std::fs::metadata(path.as_std_path()).is_ok() {
                        let existing_hash = fs::hash_file_sync(&path).ok()?;
                        if existing_hash == file.object_key {
                            trace!(?path, "file already exists with correct hash, skipping");
                            return None;
                        }
                    }

                    Some((artifact, (file, path)))
                })
                .fold(HashMap::<_, Vec<_>>::new, |mut acc, (artifact, entry)| {
                    acc.entry(artifact.clone()).or_default().push(entry);
                    acc
                })
                .reduce(HashMap::new, |mut acc, item| {
                    acc.extend(item);
                    acc
                })
        })
        .await
        .context("file validation task")
    }

    /// Rewrite file contents before storing in CAS to normalize paths.
    #[instrument(name = "CargoCache::rewrite_for_storage", skip(content))]
    async fn rewrite(ws: &Workspace, path: &AbsFilePath, content: &[u8]) -> Result<Vec<u8>> {
        // Determine what kind of file this is based on path structure.
        let components = path.component_strs_lossy().collect::<Vec<_>>();

        // Look at the last few components to determine file type.
        // We use .rev() to start from the filename and work backwards.
        let file_type = components
            .iter()
            .rev()
            .tuple_windows::<(_, _, _)>()
            .find_map(|(name, parent, gparent)| {
                let ext = name.as_ref().rsplit_once('.').map(|(_, ext)| ext);
                match (gparent.as_ref(), parent.as_ref(), name.as_ref(), ext) {
                    ("build", _, "output", _) => Some("build-script-output"),
                    ("build", _, "root-output", _) => Some("root-output"),
                    (_, _, _, Some("d")) => Some("dep-info"),
                    _ => None,
                }
            });

        match file_type {
            Some("root-output") => {
                trace!(?path, "rewriting root-output file");
                let parsed = RootOutput::from_file(ws, path).await?;
                serde_json::to_vec(&parsed).context("serialize RootOutput")
            }
            Some("build-script-output") => {
                trace!(?path, "rewriting build-script-output file");
                let parsed = BuildScriptOutput::from_file(ws, path).await?;
                serde_json::to_vec(&parsed).context("serialize BuildScriptOutput")
            }
            Some("dep-info") => {
                trace!(?path, "rewriting dep-info file");
                let parsed = DepInfo::from_file(ws, path).await?;
                serde_json::to_vec(&parsed).context("serialize DepInfo")
            }
            None => {
                // No rewriting needed, store as-is.
                Ok(content.to_vec())
            }
            Some(unknown) => {
                bail!("unknown file type for rewriting: {unknown}")
            }
        }
    }

    /// Collect library files and their fingerprints for an artifact.
    async fn collect_library_files(&self, artifact: &BuiltArtifact) -> Result<Vec<AbsFilePath>> {
        let lib_fingerprint_dir = self.ws.profile_dir.try_join_dirs(&[
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
            .iter()
            .cloned()
            .chain(lib_fingerprint_files)
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// Collect build script files and their fingerprints for an artifact.
    async fn collect_build_script_files(
        &self,
        artifact: &BuiltArtifact,
    ) -> Result<Vec<AbsFilePath>> {
        let Some(ref build_script_files) = artifact.build_script_files else {
            return Ok(vec![]);
        };

        let compiled_files = fs::walk_files(&build_script_files.compiled_dir)
            .try_collect::<Vec<_>>()
            .await?;
        let compiled_fingerprint_dir = self.ws.profile_dir.try_join_dirs(&[
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
        let output_fingerprint_dir = self.ws.profile_dir.try_join_dirs(&[
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
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// Process files for upload: read, rewrite, calculate keys, and prepare
    /// metadata.
    async fn process_files_for_upload(
        &self,
        files: Vec<AbsFilePath>,
        restored: &RestoreState,
    ) -> Result<(
        Vec<(QualifiedPath, Key)>,
        Vec<ArtifactFile>,
        Vec<(Key, Vec<u8>, AbsFilePath)>,
    )> {
        let mut library_unit_files = vec![];
        let mut artifact_files = vec![];
        let mut bulk_entries = vec![];

        for path in files {
            let Some(content) = fs::read_buffered(&path).await? else {
                warn!("failed to read file: {}", path);
                continue;
            };

            let content = Self::rewrite(&self.ws, &path, &content).await?;
            let key = Key::from_buffer(&content);

            let metadata = fs::Metadata::from_file(&path)
                .await?
                .ok_or_eyre("could not stat file metadata")?;
            let mtime_nanos = metadata.mtime.duration_since(UNIX_EPOCH)?.as_nanos();
            let qualified = QualifiedPath::parse(&self.ws, path.as_std_path()).await?;

            library_unit_files.push((qualified.clone(), key.clone()));
            artifact_files.push(
                ArtifactFile::builder()
                    .object_key(key.clone())
                    .path(serde_json::to_string(&qualified)?)
                    .mtime_nanos(mtime_nanos)
                    .executable(metadata.executable)
                    .build(),
            );

            if restored.check_object(&key) {
                trace!(?path, ?key, "skipping backup: file was restored from cache");
            } else {
                bulk_entries.push((key, content, path));
            }
        }

        Ok((library_unit_files, artifact_files, bulk_entries))
    }

    /// Upload files in bulk and return the number of bytes transferred.
    async fn upload_files_bulk(
        &self,
        bulk_entries: Vec<(Key, Vec<u8>, AbsFilePath)>,
        progress: &TransferBar,
    ) -> Result<u64> {
        if bulk_entries.is_empty() {
            return Ok(0);
        }

        debug!(count = bulk_entries.len(), "uploading files");

        let result = bulk_entries
            .iter()
            .map(|(key, content, _)| (key.clone(), content.clone()))
            .collect::<Vec<_>>()
            .pipe(stream::iter)
            .pipe(|stream| self.cas.store_bulk(stream))
            .await
            .context("upload batch")?;

        let mut uploaded_bytes = 0u64;
        for (key, content, path) in &bulk_entries {
            if result.written.contains(key) {
                uploaded_bytes += content.len() as u64;
                debug!(?path, ?key, "uploaded via bulk");
            } else if result.skipped.contains(key) {
                debug!(?path, ?key, "skipped by server (already exists)");
            }
            progress.add_files(1);
        }

        for error in &result.errors {
            warn!(
                key = ?error.key,
                error = %error.error,
                "failed to upload file in bulk operation"
            );
        }

        Ok(uploaded_bytes)
    }

    /// Spawn worker tasks to restore files from CAS in batches.
    fn spawn_restore_workers(
        &self,
        worker_count: usize,
        rx: flume::Receiver<(ArtifactFile, AbsFilePath)>,
        progress: &TransferBar,
        restored: &RestoreState,
    ) -> JoinSet<()> {
        let mut workers = JoinSet::new();
        for _ in 0..worker_count {
            let rx = rx.clone();
            let cache = self.clone();
            let progress = progress.clone();
            let restored = restored.clone();
            workers.spawn(async move {
                const BATCH_SIZE: usize = 50;
                let mut batch = Vec::new();

                while let Ok(file) = rx.recv_async().await {
                    batch.push(file);
                    if batch.len() < BATCH_SIZE {
                        continue;
                    }

                    let restore = cache
                        .process_restore_batch(&batch, &progress, &restored)
                        .await;
                    if let Err(error) = restore {
                        warn!(?error, "failed to process batch");
                    }

                    batch.clear();
                }

                let restore = cache
                    .process_restore_batch(&batch, &progress, &restored)
                    .await;
                if let Err(error) = restore {
                    warn!(?error, "failed to process final batch");
                }
            });
        }
        workers
    }

    /// Process a batch of files to restore from CAS.
    async fn process_restore_batch(
        &self,
        batch: &[(ArtifactFile, AbsFilePath)],
        progress: &TransferBar,
        restored: &RestoreState,
    ) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        let keys = batch
            .iter()
            .map(|(file, _)| file.object_key.clone())
            .collect::<Vec<_>>();

        let mut contents_stream = self.cas.get_bulk(keys).await?;
        let mut contents = HashMap::new();
        while let Some(result) = contents_stream.next().await {
            match result {
                Ok((key, data)) => {
                    contents.insert(key, data);
                }
                Err(error) => {
                    warn!(?error, "failed to fetch blob from bulk stream");
                }
            }
        }

        for (file, path) in batch {
            let Some(data) = contents.get(&file.object_key) else {
                warn!(?file, "file not found in bulk response");
                continue;
            };

            match self.restore_single_file(file, path, data, restored).await {
                Ok(transferred) => {
                    progress.add_files(1);
                    progress.add_bytes(transferred);
                }
                Err(error) => {
                    warn!(?error, ?file, "failed to restore file");
                }
            }
        }

        Ok(())
    }

    /// Restore a single file from CAS data.
    async fn restore_single_file(
        &self,
        file: &ArtifactFile,
        path: &AbsFilePath,
        data: &[u8],
        restored: &RestoreState,
    ) -> Result<u64> {
        let data = Self::reconstruct(&self.ws.profile_dir, &self.ws.cargo_home, path, data).await?;

        let mtime = UNIX_EPOCH + Duration::from_nanos(file.mtime_nanos as u64);
        let metadata = fs::Metadata::builder()
            .mtime(mtime)
            .executable(file.executable)
            .build();
        fs::write(path, &data).await?;
        metadata.set_file(path).await?;
        restored.record_object(&file.object_key);
        Ok(data.len() as u64)
    }

    /// Reconstruct file contents after retrieving from CAS.
    #[instrument(name = "CargoCache::reconstruct_from_storage", skip(content))]
    async fn reconstruct(
        profile_root: &AbsDirPath,
        cargo_home: &AbsDirPath,
        path: &AbsFilePath,
        content: &[u8],
    ) -> Result<Vec<u8>> {
        // Determine what kind of file this is based on path structure.
        let components = path.component_strs_lossy().collect::<Vec<_>>();

        // Look at the last few components to determine file type.
        let file_type = components
            .iter()
            .rev()
            .tuple_windows::<(_, _, _)>()
            .find_map(|(name, parent, gparent)| {
                let ext = name.as_ref().rsplit_once('.').map(|(_, ext)| ext);
                match (gparent.as_ref(), parent.as_ref(), name.as_ref(), ext) {
                    ("build", _, "output", _) => Some("build-script-output"),
                    ("build", _, "root-output", _) => Some("root-output"),
                    (_, _, _, Some("d")) => Some("dep-info"),
                    _ => None,
                }
            });

        match file_type {
            Some("root-output") => {
                trace!(?path, "reconstructing root-output file");
                let parsed = serde_json::from_slice::<RootOutput>(content)?;
                Ok(parsed
                    .reconstruct_raw(profile_root, cargo_home)
                    .into_bytes())
            }
            Some("build-script-output") => {
                trace!(?path, "reconstructing build-script-output file");
                let parsed = serde_json::from_slice::<BuildScriptOutput>(content)?;
                Ok(parsed
                    .reconstruct_raw(profile_root, cargo_home)
                    .into_bytes())
            }
            Some("dep-info") => {
                trace!(?path, "reconstructing dep-info file");
                let parsed = serde_json::from_slice::<DepInfo>(content)?;
                Ok(parsed
                    .reconstruct_raw(profile_root, cargo_home)
                    .into_bytes())
            }
            None => {
                // No reconstruction needed, use as-is.
                Ok(content.to_vec())
            }
            Some(unknown) => {
                bail!("unknown file type for reconstruction: {unknown}")
            }
        }
    }
}

/// Calculate content hash for a library unit from its files.
fn calculate_content_hash(library_unit_files: Vec<(QualifiedPath, Key)>) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let bytes = serde_json::to_vec(&LibraryUnitHash::new(library_unit_files))?;
    hasher.write_all(&bytes)?;
    hasher.finalize().to_hex().to_string().pipe(Ok)
}

/// Build a CargoSaveRequest from artifact data.
fn build_save_request(
    artifact: &BuiltArtifact,
    target: &str,
    content_hash: String,
    artifact_files: Vec<ArtifactFile>,
) -> CargoSaveRequest {
    CargoSaveRequest::builder()
        .package_name(&artifact.package_name)
        .package_version(&artifact.package_version)
        .target(target)
        .library_crate_compilation_unit_hash(&artifact.library_crate_compilation_unit_hash)
        .maybe_build_script_compilation_unit_hash(
            artifact.build_script_compilation_unit_hash.as_ref(),
        )
        .maybe_build_script_execution_unit_hash(artifact.build_script_execution_unit_hash.as_ref())
        .content_hash(content_hash)
        .artifacts(artifact_files)
        .build()
}

/// Build CargoRestoreRequest objects from an artifact plan.
fn build_restore_requests(
    artifact_plan: &ArtifactPlan,
) -> (HashMap<Vec<u8>, &ArtifactKey>, Vec<CargoRestoreRequest>) {
    artifact_plan.artifacts.iter().fold(
        (HashMap::new(), Vec::new()),
        |(mut artifacts, mut requests), artifact| {
            let req = CargoRestoreRequest::builder()
                .package_name(&artifact.package_name)
                .package_version(&artifact.package_version)
                .target(&artifact_plan.target)
                .library_crate_compilation_unit_hash(&artifact.library_crate_compilation_unit_hash)
                .maybe_build_script_compilation_unit_hash(
                    artifact.build_script_compilation_unit_hash.as_ref(),
                )
                .maybe_build_script_execution_unit_hash(
                    artifact.build_script_execution_unit_hash.as_ref(),
                )
                .build();
            artifacts.insert(req.hash(), artifact);
            requests.push(req);
            (artifacts, requests)
        },
    )
}

/// An ArtifactPlan represents the collection of information known about the
/// artifacts for a build statically at compile-time.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
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
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArtifactKey {
    // Partial artifact key information. Note that this is only derived from the
    // build plan, and therefore is missing essential information (e.g. `rustc`
    // flags from build script output directives) that can only be determined
    // interactively.
    //
    // TODO: There are more fields here that we can know from the planning stage
    // that need to be added (e.g. features).
    package_name: String,
    package_version: String,

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

    lib_files: Vec<AbsFilePath>,
    build_script_files: Option<BuildScriptDirs>,

    library_crate_compilation_unit_hash: String,
    build_script_compilation_unit_hash: Option<String>,
    build_script_execution_unit_hash: Option<String>,

    // TODO: Should these all be in a larger `BuildScript` struct that includes
    // the files, unit hashes, and output? It's a little silly to all have them
    // be separately optional, as if we could have some fields but not others.
    build_script_output: Option<BuildScriptOutput>,
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

    /// Reconstruct a representative `ArtifactKey`.
    pub fn reconstruct_key(&self) -> ArtifactKey {
        ArtifactKey {
            package_name: self.package_name.clone(),
            package_version: self.package_version.clone(),
            lib_files: self.lib_files.clone(),
            build_script_files: self.build_script_files.clone(),
            library_crate_compilation_unit_hash: self.library_crate_compilation_unit_hash.clone(),
            build_script_compilation_unit_hash: self.build_script_compilation_unit_hash.clone(),
            build_script_execution_unit_hash: self.build_script_execution_unit_hash.clone(),
        }
    }
}

/// A content hash of a library unit's artifacts.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize)]
struct LibraryUnitHash {
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
    fn new(mut files: Vec<(QualifiedPath, Key)>) -> Self {
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
