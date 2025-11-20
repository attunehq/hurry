use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _, bail},
};
use dashmap::DashSet;
use derive_more::Debug;
use futures::{StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::task::JoinSet;
use tracing::{Instrument, debug, instrument, trace, warn};

use tap::Pipe as _;

use crate::{
    cargo::{self, QualifiedPath, UnitPlan, Workspace, cache, workspace::UnitHash},
    cas::CourierCas,
    fs,
    path::{AbsFilePath, JoinWith as _},
    progress::TransferBar,
};
use clients::{
    Courier,
    courier::v1::{
        Key, SavedUnit,
        cache::{CargoRestoreRequest2, SavedUnitCacheKey},
    },
};

/// Tracks items that were restored from the cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Restored {
    /// Stores the unit hashes of restored units.
    pub units: DashSet<UnitHash>,
    pub files: DashSet<Key>,
}

impl Restored {
    /// Records that an artifact was restored from cache.
    fn record_unit(&self, unit_hash: UnitHash) {
        self.units.insert(unit_hash);
    }

    /// Records that an object was restored from cache.
    fn record_file(&self, key: Key) {
        self.files.insert(key);
    }
}

#[derive(Debug)]
struct FileRestoreKey {
    key: Key,
    #[debug(skip)]
    write: Box<dyn FnOnce(&Vec<u8>) -> BoxFuture<'static, Result<()>> + Send + Sync>,
}

#[instrument(skip(units, progress))]
pub async fn restore_units(
    courier: &Courier,
    cas: &CourierCas,
    ws: &Workspace,
    units: &Vec<UnitPlan>,
    // artifact_plan: &ArtifactPlan,
    progress: &TransferBar,
) -> Result<Restored> {
    trace!(?units, "units");

    let restored = Restored::default();

    // TODO: Check which units are already fresh on disk, and don't attempt to
    // restore them.

    // Load unit information from remote cache. Note that this does NOT download
    // the actual files, which are loaded as CAS keys.
    let bulk_req = CargoRestoreRequest2::new(units.iter().map(|unit| {
        SavedUnitCacheKey::builder()
            .unit_hash(unit.info().unit_hash.clone())
            .build()
    }));
    let mut saved_units = courier.cargo_cache_restore2(bulk_req).await?;

    // Spawn concurrent workers for doing parallel downloads.
    let (tx, mut workers) = {
        let worker_count = num_cpus::get();
        // We use an unbounded channel here because if we use a bounded channel,
        // errors in the client then (incorrectly) get clobbered by the error
        // caused by sending to a closed channel. We already buffer the entire
        // set of work items we want to send, so using an unbounded channel for
        // it doesn't cause additional memory pressure- we just move our
        // buffered set of work items into the channel all at once instead of as
        // they're being worked on.
        let (tx, rx) = flume::unbounded::<FileRestoreKey>();
        let mut workers = JoinSet::new();
        for worker_id in 0..worker_count {
            let rx = rx.clone();
            let cas = cas.clone();
            let progress = progress.clone();
            let restored = restored.clone();
            let span = tracing::info_span!("restore_worker", worker_id);
            workers.spawn(restore_worker(rx, cas, progress, restored).instrument(span));
        }
        // Dropping the `rx` causes it to close, so we cannot drop it until all
        // workers have finished receiving files.
        (tx, workers)
    };

    let mut dep_fingerprints = HashMap::new();
    let mut files_to_restore = Vec::<FileRestoreKey>::new();
    for unit in units {
        let unit_hash = unit.info().unit_hash.clone();

        // Load the saved file info from the response.
        let saved = saved_units.take(
            &SavedUnitCacheKey::builder()
                .unit_hash(unit_hash.clone())
                .build(),
        );
        let Some(saved) = saved else {
            debug!(?unit_hash, "unit missing from cache");
            progress.dec_length(1);
            continue;
        };

        // Write the fingerprint. This happens during this loop because
        // fingerprint rewriting must occur in dependency order.
        //
        // TODO: Ideally, we would only write fingerprints _after_ all the files
        // for the unit are restored, to be maximally correct. This requires
        // plumbing some sort of work-dependency relationship between units and
        // restores.
        match (saved, unit) {
            (
                SavedUnit::LibraryCrate(saved_library_files, _),
                UnitPlan::LibraryCrate(unit_plan),
            ) => {
                // Restore the fingerprint directly, because fingerprint
                // rewriting needs to occur in dependency order.
                let fingerprint: cargo::Fingerprint =
                    serde_json::from_str(saved_library_files.fingerprint.as_str())?;
                cache::LibraryFiles::restore_fingerprint(
                    ws,
                    &mut dep_fingerprints,
                    fingerprint,
                    unit_plan,
                )
                .await?;

                // Queue the output files.
                for file in saved_library_files.output_files {
                    let path: QualifiedPath = serde_json::from_str(file.path.as_str())?;
                    let ws_clone = ws.clone();
                    let target_arch = unit_plan.info.target_arch.clone();
                    let executable = file.executable;

                    files_to_restore.push(FileRestoreKey {
                        key: file.object_key.clone(),
                        write: Box::new(move |data| {
                            let data = data.clone();
                            Box::pin(async move {
                                let abs_path = path
                                    .reconstruct(&ws_clone, &target_arch)?
                                    .pipe(AbsFilePath::try_from)?;
                                fs::write(&abs_path, data).await?;
                                fs::set_executable(&abs_path, executable).await?;
                                Ok(())
                            })
                        }),
                    });
                }

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                // Queue the dep-info file with reconstruction.
                let ws_clone = ws.clone();
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: saved_library_files.dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let dep_info: cargo::DepInfo = serde_json::from_slice(&data)?;
                            let reconstructed = dep_info
                                .reconstruct(&ws_clone, &unit_plan_clone.info.target_arch)?;
                            fs::write(
                                &profile_dir_clone.join(&unit_plan_clone.dep_info_file()?),
                                reconstructed,
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });

                // Queue the encoded dep-info file (no transformation).
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: saved_library_files.encoded_dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(
                                &profile_dir_clone.join(&unit_plan_clone.encoded_dep_info_file()?),
                                data,
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });
            }
            (
                SavedUnit::BuildScriptCompilation(build_script_compiled_files, _),
                UnitPlan::BuildScriptCompilation(unit_plan),
            ) => {
                // Restore the fingerprint directly, because fingerprint
                // rewriting needs to occur in dependency order.
                let fingerprint: cargo::Fingerprint =
                    serde_json::from_str(build_script_compiled_files.fingerprint.as_str())?;
                cache::BuildScriptCompiledFiles::restore_fingerprint(
                    ws,
                    &mut dep_fingerprints,
                    fingerprint,
                    unit_plan,
                )
                .await?;

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                // Queue compiled program with hard link creation.
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: build_script_compiled_files.compiled_program.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let program_file =
                                profile_dir_clone.join(unit_plan_clone.program_file()?);
                            fs::write(&program_file, data).await?;
                            fs::set_executable(&program_file, true).await?;
                            fs::hard_link(
                                &program_file,
                                &profile_dir_clone.join(unit_plan_clone.linked_program_file()?),
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });

                // Queue dep-info file with reconstruction.
                let ws_clone = ws.clone();
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: build_script_compiled_files.dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let dep_info: cargo::DepInfo = serde_json::from_slice(&data)?;
                            let reconstructed = dep_info
                                .reconstruct(&ws_clone, &unit_plan_clone.info.target_arch)?;
                            fs::write(
                                &profile_dir_clone.join(&unit_plan_clone.dep_info_file()?),
                                reconstructed,
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });

                // Queue encoded dep-info file (no transformation).
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: build_script_compiled_files.encoded_dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(
                                &profile_dir_clone.join(&unit_plan_clone.encoded_dep_info_file()?),
                                data,
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });
            }
            (
                SavedUnit::BuildScriptExecution(build_script_output_files, _),
                UnitPlan::BuildScriptExecution(unit_plan),
            ) => {
                // Restore the fingerprint directly, because fingerprint
                // rewriting needs to occur in dependency order.
                let fingerprint: cargo::Fingerprint =
                    serde_json::from_str(build_script_output_files.fingerprint.as_str())?;
                cache::BuildScriptOutputFiles::restore_fingerprint(
                    ws,
                    &mut dep_fingerprints,
                    fingerprint,
                    unit_plan,
                )
                .await?;

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                // Queue all OUT_DIR files with executable flag handling.
                for file in build_script_output_files.out_dir_files {
                    let path: QualifiedPath = serde_json::from_str(file.path.as_str())?;
                    let ws_clone = ws.clone();
                    let target_arch = unit_plan.info.target_arch.clone();
                    let executable = file.executable;

                    files_to_restore.push(FileRestoreKey {
                        key: file.object_key.clone(),
                        write: Box::new(move |data| {
                            let data = data.clone();
                            Box::pin(async move {
                                let abs_path = path
                                    .reconstruct(&ws_clone, &target_arch)?
                                    .pipe(AbsFilePath::try_from)?;
                                fs::write(&abs_path, data).await?;
                                fs::set_executable(&abs_path, executable).await?;
                                Ok(())
                            })
                        }),
                    });
                }

                // Queue stdout with BuildScriptOutput reconstruction.
                let ws_clone = ws.clone();
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: build_script_output_files.stdout.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let stdout: cargo::BuildScriptOutput = serde_json::from_slice(&data)?;
                            let reconstructed =
                                stdout.reconstruct(&ws_clone, &unit_plan_clone.info.target_arch)?;
                            fs::write(
                                &profile_dir_clone.join(&unit_plan_clone.stdout_file()?),
                                reconstructed,
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });

                // Queue stderr (no transformation).
                let unit_plan_clone = unit_plan.clone();
                let profile_dir_clone = profile_dir.clone();
                files_to_restore.push(FileRestoreKey {
                    key: build_script_output_files.stderr.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(
                                &profile_dir_clone.join(&unit_plan_clone.stderr_file()?),
                                data,
                            )
                            .await?;
                            Ok(())
                        })
                    }),
                });

                // Generate root-output file (not from CAS - synthesized from unit_plan).
                let root_output_path = profile_dir.join(&unit_plan.root_output_file()?);
                fs::write(
                    &root_output_path,
                    unit_plan.out_dir()?.as_os_str().as_encoded_bytes(),
                )
                .await?;
            }
            _ => bail!("unit type mismatch"),
        }

        // Queue the other files in the unit to be batch downloaded and
        // restored.

        // Mark the unit as restored. It's not _technically_ restored yet, but
        // this function will return an error if the restore doesn't happen
        // anyway.
        restored.record_unit(unit_hash);
        // TODO: Ideally we would not increment this until the restore is
        // actually finished, but we haven't plumbed that through yet.
        progress.inc(1);
    }

    debug!("start sending files to restore workers");
    for file in files_to_restore {
        tx.send_async(file).await?;
        progress.add_files(1);
    }
    drop(tx);
    debug!("done sending files to restore workers");

    debug!("start joining restore workers");
    while let Some(worker) = workers.join_next().await {
        worker
            .context("could not join worker")?
            .context("worker returned an error")?;
    }
    debug!("done joining restore workers");

    Ok(restored)
}

async fn restore_worker(
    rx: flume::Receiver<FileRestoreKey>,
    cas: CourierCas,
    progress: TransferBar,
    restored: Restored,
) -> Result<()> {
    const BATCH_SIZE: usize = 50;
    let mut batch = Vec::new();
    while let Ok(file) = rx.recv_async().await {
        debug!(?file, "worker got file");

        // Add the file to the batch.
        batch.push(file);
        debug!(len = ?batch.len(), "batch length");
        // If the batch is not full, wait for the batch to fill.
        if batch.len() < BATCH_SIZE {
            debug!("batch not full, waiting for more files");
            continue;
        }
        debug!("batch full, restoring");

        // Restore batches once full.
        let batch_to_restore = std::mem::take(&mut batch);
        restore_batch(batch_to_restore, &cas, &progress, &restored).await?;
    }
    debug!("worker rx closed");

    // Once the channel closes, there may still be a partially filled batch
    // remaining. Restore the remaining files in the batch.
    if !batch.is_empty() {
        debug!(?batch, "restoring remaining batch");
        restore_batch(batch, &cas, &progress, &restored).await?;
        debug!("done restoring remaining batch");
    }

    Ok(())
}

#[instrument(skip_all)]
async fn restore_batch(
    batch: Vec<FileRestoreKey>,
    cas: &CourierCas,
    progress: &TransferBar,
    restored: &Restored,
) -> Result<()> {
    debug!(?batch, "restoring batch");

    // Note that you can have multiple files with the same key.
    // Build a map of object keys to file restore keys.
    let mut key_to_files = HashMap::new();
    for file in batch {
        key_to_files
            .entry(file.key.clone())
            .or_insert(vec![])
            .push(file);
    }

    // Now that keys are deduplicated, we can send them to the CAS; this way we
    // avoid making the server send multiple copies of the same file content.
    let keys = key_to_files.keys().cloned().collect::<Vec<_>>();

    // For each streamed CAS key, restore the file to the local filesystem.
    debug!(?keys, "start fetching files from CAS");
    let mut res = cas.get_bulk(keys).await?;
    debug!("start streaming response from CAS");
    while let Some(result) = res.next().await {
        match result {
            Ok((key, data)) => {
                debug!(?key, "CAS stream entry");
                let files = key_to_files
                    .remove(&key)
                    .ok_or_eyre("unrecognized key from CAS bulk response")?;
                for file in files {
                    restored.record_file(file.key);

                    progress.add_files(1);
                    progress.add_bytes(data.len() as u64);

                    // Call the write callback to handle all file operations.
                    debug!(?key, "calling write callback");
                    (file.write)(&data).await?;
                    debug!(?key, "done calling write callback");
                }
            }
            Err(error) => {
                warn!(?error, "failed to fetch file from CAS");
            }
        }
    }
    debug!("done streaming response from CAS");

    Ok(())
}
