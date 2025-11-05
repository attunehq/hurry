use color_eyre::{
    Result,
    eyre::{Context as _, bail},
};
use dashmap::DashSet;
use derive_more::Debug;
use futures::StreamExt;
use itertools::Itertools;
use std::{
    collections::HashMap,
    time::{Duration, UNIX_EPOCH},
};
use tap::Pipe as _;
use tokio::task::JoinSet;
use tracing::{debug, instrument, trace, warn};

use crate::{
    cargo::{
        ArtifactKey, ArtifactPlan, BuildScriptOutput, DepInfo, QualifiedPath, RootOutput, Workspace,
    },
    cas::CourierCas,
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

/// Tracks items that were restored from the cache.
#[derive(Debug, Clone, Default)]
pub struct Restored {
    pub artifacts: DashSet<ArtifactKey>,
    pub objects: DashSet<Key>,
}

impl Restored {
    /// Records that an artifact was restored from cache.
    fn record_artifact(&self, artifact: &ArtifactKey) {
        self.artifacts.insert(artifact.clone());
    }

    /// Records that an object was restored from cache.
    fn record_object(&self, key: &Key) {
        self.objects.insert(key.clone());
    }
}

#[instrument(skip(artifact_plan, progress))]
pub async fn restore_artifacts(
    courier: &Courier,
    cas: &CourierCas,
    ws: &Workspace,
    artifact_plan: &ArtifactPlan,
    progress: &TransferBar,
) -> Result<Restored> {
    trace!(?artifact_plan, "artifact plan");
    let (artifacts, requests) = build_restore_requests(artifact_plan);
    let restore_result = courier
        .cargo_cache_restore_bulk(requests)
        .await
        .context("cache restore")?;
    trace!(?restore_result, "cache restore response");

    for miss in restore_result.misses {
        debug!(artifact = ?miss, "no matching library unit build found");
        progress.dec_length(1);
    }
    let files_to_restore = filter_files_need_restored(ws, restore_result.hits, artifacts).await?;
    trace!(?files_to_restore, "files to restore");

    let restored = Restored::default();
    let worker_count = num_cpus::get();
    let (tx, rx) = flume::bounded::<(ArtifactFile, AbsFilePath)>(0);
    let mut workers = spawn_restore_workers(cas, ws, worker_count, rx.clone(), progress, &restored);
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

    Ok(restored)
}

/// Filter the set to only the files which need to be restored, either
/// because they don't exist locally or their hashes don't match.
#[instrument(skip(hits, artifacts))]
async fn filter_files_need_restored(
    ws: &Workspace,
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
                .reconstruct_raw(&ws.profile_dir, &ws.cargo_home)
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
#[instrument(skip(restored))]
fn spawn_restore_workers(
    cas: &CourierCas,
    ws: &Workspace,
    worker_count: usize,
    rx: flume::Receiver<(ArtifactFile, AbsFilePath)>,
    progress: &TransferBar,
    restored: &Restored,
) -> JoinSet<()> {
    let mut workers = JoinSet::new();
    for _ in 0..worker_count {
        let rx = rx.clone();
        let cas = cas.clone();
        let ws = ws.clone();
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

                let restore = process_restore_batch(&cas, &ws, &batch, &progress, &restored).await;
                if let Err(error) = restore {
                    warn!(?error, "failed to process batch");
                }

                batch.clear();
            }

            let restore = process_restore_batch(&cas, &ws, &batch, &progress, &restored).await;
            if let Err(error) = restore {
                warn!(?error, "failed to process final batch");
            }
        });
    }
    workers
}

/// Process a batch of files to restore from CAS.
#[instrument(skip(restored))]
async fn process_restore_batch(
    cas: &CourierCas,
    ws: &Workspace,
    batch: &[(ArtifactFile, AbsFilePath)],
    progress: &TransferBar,
    restored: &Restored,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }

    let keys = batch
        .iter()
        .map(|(file, _)| file.object_key.clone())
        .collect::<Vec<_>>();

    let mut contents_stream = cas.get_bulk(keys).await?;
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

        match restore_single_file(ws, file, path, data, restored).await {
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
#[instrument(skip(data, restored))]
async fn restore_single_file(
    ws: &Workspace,
    file: &ArtifactFile,
    path: &AbsFilePath,
    data: &[u8],
    restored: &Restored,
) -> Result<u64> {
    let data = reconstruct(&ws.profile_dir, &ws.cargo_home, path, data).await?;

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
#[instrument(skip(content))]
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
