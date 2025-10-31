use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context as _, OptionExt, bail, eyre},
};
use dashmap::DashSet;
use derive_more::Debug;
use futures::StreamExt;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env::VarError,
    process::Stdio,
    time::{Duration, UNIX_EPOCH},
};
use tap::Pipe as _;
use tokio::{io::AsyncBufReadExt as _, task::JoinSet};
use tracing::{debug, instrument, trace, warn};
use url::Url;
use uuid::Uuid;

use crate::{
    cargo::{
        ArtifactKey, ArtifactPlan, BuildScriptOutput, DepInfo, QualifiedPath, RootOutput, Workspace,
    },
    cas::CourierCas,
    daemon::{
        CargoUploadRequest, CargoUploadStatus, CargoUploadStatusRequest, CargoUploadStatusResponse,
        DaemonPaths, DaemonReadyMessage,
    },
    fs,
    path::{AbsDirPath, AbsFilePath},
    progress::TransferBar,
};
use clients::{
    Courier,
    courier::v1::{
        Key,
        cache::{ArtifactFile, CargoBulkRestoreHit, CargoRestoreRequest},
    },
};

/// Statistics about cache operations.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
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
    #[debug("{:?}", courier_url.as_str())]
    courier_url: Url,
    courier: Courier,
    cas: CourierCas,
    ws: Workspace,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open")]
    pub async fn open(courier_url: Url, ws: Workspace) -> Result<Self> {
        let courier = Courier::new(courier_url.clone())?;
        courier.ping().await.context("ping courier service")?;
        let cas = CourierCas::new(courier.clone());
        Ok(Self {
            courier_url,
            courier,
            cas,
            ws,
        })
    }

    #[instrument(name = "CargoCache::save", skip(artifact_plan, restored))]
    pub async fn save(&self, artifact_plan: ArtifactPlan, restored: RestoreState) -> Result<Uuid> {
        trace!(?artifact_plan, "artifact plan");
        let paths = DaemonPaths::initialize().await?;

        // Start daemon if it's not already running. If it is, try to read its context
        // file to get its url, which we need to know in order to communicate with it.
        let daemon = if paths.daemon_running().await? {
            paths
                .read_context()
                .await?
                .ok_or_eyre("daemon running but no context file")?
        } else {
            // TODO: Ideally we'd replace this with proper double-fork daemonization to
            // avoid the security and compatibility concerns here: someone could replace the
            // binary at this path in the time between when this binary launches and when it
            // re-launches itself as a daemon.
            let hurry_binary = std::env::current_exe().context("read current binary path")?;

            // Spawn self as a child and wait for the ready message on STDOUT.
            let mut cmd = tokio::process::Command::new(hurry_binary);
            cmd.arg("daemon")
                .arg("start")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // If `HURRY_LOG` is not set, set it to `debug` by default so the
            // logs are useful.
            if let Err(VarError::NotPresent) = std::env::var("HURRY_LOG") {
                cmd.env("HURRY_LOG", "debug");
            }

            let mut child = cmd.spawn()?;
            let stdout = child.stdout.take().ok_or_eyre("daemon has no stdout")?;
            let mut stdout = tokio::io::BufReader::new(stdout).lines();

            // This value was chosen arbitrarily. Adjust as needed.
            const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
            let line = tokio::time::timeout(DAEMON_STARTUP_TIMEOUT, stdout.next_line())
                .await
                .map_err(|elapsed| eyre!("daemon startup timed out after {elapsed:?}"))?
                .context("read daemon output")?
                .ok_or_eyre("daemon crashed on startup")?;
            serde_json::from_str::<DaemonReadyMessage>(&line)
                .context("parse daemon ready message")
                .with_section(|| line.header("Daemon output:"))?
        };

        // Connect to daemon HTTP server.
        let client = reqwest::Client::default();
        let endpoint = format!("http://{}/api/v0/cargo/upload", daemon.url);

        // Send upload request.
        let request_id = Uuid::new_v4();
        let request = CargoUploadRequest {
            request_id,
            courier_url: self.courier_url.clone(),
            ws: self.ws.clone(),
            artifact_plan,
            skip_artifacts: restored.artifacts.into_iter().collect(),
            skip_objects: restored.objects.into_iter().collect(),
        };
        trace!(?request, "submitting upload request");
        let response = client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("send upload request to daemon at: {endpoint}"))
            .with_section(|| format!("{daemon:?}").header("Daemon context:"))?;
        trace!(?response, "got upload response");

        Ok(request_id)
    }

    #[instrument(name = "CargoCache::wait_for_upload")]
    pub async fn wait_for_upload(&self, request_id: &Uuid, progress: &TransferBar) -> Result<()> {
        let paths = DaemonPaths::initialize().await?;
        let daemon = if paths.daemon_running().await? {
            let context = fs::read_buffered_utf8(&paths.context_path)
                .await
                .context("read daemon context file")?
                .ok_or_eyre("no daemon context file")?;
            serde_json::from_str::<DaemonReadyMessage>(&context)
                .context("parse daemon context")
                .with_section(|| context.header("Daemon context file:"))?
        } else {
            bail!("daemon is not running");
        };

        let client = reqwest::Client::default();
        let endpoint = format!("http://{}/api/v0/cargo/status", daemon.url);
        let request = CargoUploadStatusRequest {
            request_id: *request_id,
        };
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        let mut last_uploaded_artifacts = 0u64;
        let mut last_uploaded_files = 0u64;
        let mut last_uploaded_bytes = 0u64;
        let mut last_total_artifacts = 0u64;
        loop {
            interval.tick().await;
            trace!(?request, "submitting upload status request");
            let response = client
                .post(&endpoint)
                .json(&request)
                .send()
                .await
                .with_context(|| format!("send upload status request to daemon at: {endpoint}"))
                .with_section(|| format!("{daemon:?}").header("Daemon context:"))?;
            trace!(?response, "got upload status response");
            let response = response.json::<CargoUploadStatusResponse>().await?;
            trace!(?response, "parsed upload status response");
            let status = response.status.ok_or_eyre("no upload status")?;
            match status {
                CargoUploadStatus::Complete => break,
                CargoUploadStatus::InProgress {
                    uploaded_artifacts,
                    uploaded_files,
                    uploaded_bytes,
                    total_artifacts,
                } => {
                    progress.add_bytes(uploaded_bytes.saturating_sub(last_uploaded_bytes));
                    last_uploaded_bytes = uploaded_bytes;
                    progress.add_files(uploaded_files.saturating_sub(last_uploaded_files));
                    last_uploaded_files = uploaded_files;
                    progress.inc(uploaded_artifacts.saturating_sub(last_uploaded_artifacts));
                    last_uploaded_artifacts = uploaded_artifacts;
                    progress.dec_length(last_total_artifacts.saturating_sub(total_artifacts));
                    last_total_artifacts = total_artifacts;
                }
            }
        }

        Ok(())
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
        trace!(?restore_result, "cache restore response");

        for miss in restore_result.misses {
            debug!(artifact = ?miss, "no matching library unit build found");
            progress.dec_length(1);
        }
        let files_to_restore = self
            .filter_files_need_restored(restore_result.hits, artifacts)
            .await?;
        trace!(?files_to_restore, "files to restore");

        let restored = RestoreState::default();
        let worker_count = num_cpus::get();
        let (tx, rx) = flume::bounded::<(ArtifactFile, AbsFilePath)>(0);
        let mut workers = self.spawn_restore_workers(worker_count, rx.clone(), progress, &restored);
        for (artifact, files) in files_to_restore {
            for (file, path) in files {
                trace!(?artifact, ?file, ?path, "sending file to restore workers");
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
    #[instrument(name = "CargoCache::filter_files_need_restored", skip(hits, artifacts))]
    async fn filter_files_need_restored(
        &self,
        hits: Vec<CargoBulkRestoreHit>,
        artifacts: HashMap<Vec<u8>, ArtifactKey>,
    ) -> Result<HashMap<ArtifactKey, Vec<(ArtifactFile, AbsFilePath)>>> {
        let mut files_to_restore: HashMap<ArtifactKey, Vec<(ArtifactFile, AbsFilePath)>> =
            HashMap::new();
        for hit in hits {
            let Some(artifact) = artifacts.get(&hit.request.hash()) else {
                bail!("artifact was not requested but was restored: {hit:?}");
            };

            for file in hit.artifacts {
                // Convert the artifact file path back to QualifiedPath and reconstruct it to an
                // absolute path for this machine.
                let qualified = serde_json::from_str::<QualifiedPath>(&file.path)?;
                let path = qualified
                    .reconstruct_raw(&self.ws.profile_dir, &self.ws.cargo_home)
                    .pipe(AbsFilePath::try_from)?;

                // Check if file already exists with correct content. If so, don't need to
                // restore it.
                if fs::exists(path.as_std_path()).await {
                    let existing_hash = fs::hash_file(&path).await?;
                    if existing_hash == file.object_key {
                        trace!(?path, "file already exists with correct hash, skipping");
                        continue;
                    } else {
                        trace!(expected = %file.object_key, actual = %existing_hash, ?path, "file already exists, but incorrect hash");
                    }
                } else {
                    trace!(?path, "file does not exist");
                }

                files_to_restore
                    .entry(artifact.to_owned())
                    .or_default()
                    .push((file, path));
            }
        }

        Ok(files_to_restore)
    }

    /// Spawn worker tasks to restore files from CAS in batches.
    #[instrument(name = "CargoCache::spawn_restore_workers", skip(restored))]
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
                    trace!(?file, "worker got file");
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
    #[instrument(name = "CargoCache::process_restore_batch", skip(restored))]
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
    #[instrument(name = "CargoCache::restore_single_file", skip(data, restored))]
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
            .len(data.len() as u64)
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

/// Build CargoRestoreRequest objects from an artifact plan.
fn build_restore_requests(
    artifact_plan: &ArtifactPlan,
) -> (HashMap<Vec<u8>, ArtifactKey>, Vec<CargoRestoreRequest>) {
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
            artifacts.insert(req.hash(), artifact.clone());
            requests.push(req);
            (artifacts, requests)
        },
    )
}
