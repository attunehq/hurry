use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _, bail},
};
use dashmap::{DashMap, DashSet};
use derive_more::Debug;
use futures::{StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::task::JoinSet;
use tracing::{Instrument, debug, instrument, trace, warn};

use crate::{
    cargo::{self, QualifiedPath, UnitPlan, Workspace, cache, workspace::UnitHash},
    cas::CourierCas,
    fs,
    path::JoinWith as _,
    progress::TransferBar,
};
use clients::{
    Courier,
    courier::v1::{
        Key, SavedUnit,
        cache::{CargoRestoreRequest, SavedUnitCacheKey},
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
    unit_hash: UnitHash,
    key: Key,
    #[allow(
        clippy::type_complexity,
        reason = "it's a closure that returns a future of Result<()>"
    )]
    #[debug(skip)]
    write: Box<dyn FnOnce(&Vec<u8>) -> BoxFuture<'static, Result<()>> + Send + Sync>,
}

/// Tracks restore progress. It does this by tracking which units have been
/// queued for restore, and which of their files _remain_ to be restored. After
/// each file is restored, we remove it from its unit's set of pending files.
/// When the set of pending files for a unit is empty, we know that the unit has
/// been fully restored, because we added all of the unit's files to its pending
/// set before restoring any files.
#[derive(Debug, Clone, Default)]
struct RestoreProgress {
    units: Arc<DashMap<UnitHash, DashSet<Key>>>,
}

#[instrument(skip(units, progress))]
pub async fn restore_units(
    courier: &Courier,
    cas: &CourierCas,
    ws: &Workspace,
    units: &Vec<UnitPlan>,
    progress: &TransferBar,
) -> Result<Restored> {
    trace!(?units, "units");

    let restored = Restored::default();
    let start_time = SystemTime::now();

    // TODO: Check which units are already fresh on disk, and don't attempt to
    // restore them.

    // Load unit information from remote cache. Note that this does NOT download
    // the actual files, which are loaded as CAS keys.
    let bulk_req = CargoRestoreRequest::new(units.iter().map(|unit| {
        SavedUnitCacheKey::builder()
            .unit_hash(unit.info().unit_hash.clone())
            .build()
    }));
    let mut saved_units = courier.cargo_cache_restore(bulk_req).await?;

    // Track restore progress.
    let restore_progress = RestoreProgress::default();

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
            let restore_progress = restore_progress.clone();
            let span = tracing::info_span!("restore_worker", worker_id);
            workers.spawn(
                restore_worker(rx, cas, progress, restored, restore_progress).instrument(span),
            );
        }
        // Dropping the `rx` causes it to close, so we cannot drop it until all
        // workers have finished receiving files.
        (tx, workers)
    };

    let mut dep_fingerprints = HashMap::new();
    let mut files_to_restore = Vec::<FileRestoreKey>::new();
    // Shared references to clone once here instead of cloning once per unit.
    let ws = Arc::new(ws.clone());
    for (i, unit) in units.iter().enumerate() {
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

        // Calculate the mtime for files to be restored. All output file mtimes
        // for a unit U must be after those of U's dependencies (i.e. all of U's
        // mtimes must be before its dependents). To satisfy this property
        // easily, we set the mtime of all files in U to be the same, and
        // increment this mtime for every unit we see (since units are in
        // dependency order).
        //
        // We use a 1s increment here so that mtimes are still correctly set on
        // filesystems with low timestamp precision. For reference, see Cargo's
        // timestamp comparison logic.[^1]
        //
        // [^1]: https://github.com/rust-lang/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/fingerprint/mod.rs#L1229-L1235
        let mtime = start_time + Duration::from_secs(i as u64);

        // Mark the unit's restore as pending.
        restore_progress
            .units
            .insert(unit_hash.clone(), DashSet::new());
        // Write the fingerprint and queue other files to be restored. Writing
        // fingerprints happens during this loop because fingerprint rewriting
        // must occur in dependency order.
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
                    &ws,
                    &mut dep_fingerprints,
                    fingerprint,
                    unit_plan,
                )
                .await?;

                // Queue the output files.
                for file in saved_library_files.output_files {
                    let path: QualifiedPath = serde_json::from_str(file.path.as_str())?;
                    let path = path.reconstruct(&ws, &unit_plan.info).try_into()?;
                    let executable = file.executable;

                    restore_progress
                        .units
                        .get_mut(&unit_hash)
                        .ok_or_eyre("unit hash restore progress not initialized")?
                        .insert(file.object_key.clone());
                    files_to_restore.push(FileRestoreKey {
                        unit_hash: unit_hash.clone(),
                        key: file.object_key.clone(),
                        write: Box::new(move |data| {
                            let data = data.clone();
                            Box::pin(async move {
                                fs::write(&path, data).await?;
                                fs::set_executable(&path, executable).await?;
                                fs::set_mtime(&path, mtime).await?;
                                Ok(())
                            })
                        }),
                    });
                }

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                // Queue the dep-info file with reconstruction.
                let ws = ws.clone();
                let info = unit_plan.info.clone();
                let path = profile_dir.join(&unit_plan.dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(saved_library_files.dep_info_file.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: saved_library_files.dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let dep_info: cargo::DepInfo = serde_json::from_slice(&data)?;
                            let dep_info = dep_info.reconstruct(&ws, &info);
                            fs::write(&path, dep_info).await?;
                            fs::set_mtime(&path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                // Queue the encoded dep-info file (no transformation).
                let path = profile_dir.join(&unit_plan.encoded_dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(saved_library_files.encoded_dep_info_file.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: saved_library_files.encoded_dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(&path, data).await?;
                            fs::set_mtime(&path, mtime).await?;
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
                    &ws,
                    &mut dep_fingerprints,
                    fingerprint,
                    unit_plan,
                )
                .await?;

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                // Queue compiled program with hard link creation.
                let path = profile_dir.join(unit_plan.program_file()?);
                let linked_path = profile_dir.join(unit_plan.linked_program_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_compiled_files.compiled_program.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_compiled_files.compiled_program.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(&path, data).await?;
                            fs::set_executable(&path, true).await?;
                            fs::set_mtime(&path, mtime).await?;

                            fs::hard_link(&path, &linked_path).await?;
                            fs::set_mtime(&linked_path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                // Queue dep-info file with reconstruction.
                let ws = ws.clone();
                let info = unit_plan.info.clone();
                let path = profile_dir.join(&unit_plan.dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_compiled_files.dep_info_file.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_compiled_files.dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let dep_info: cargo::DepInfo = serde_json::from_slice(&data)?;
                            let dep_info = dep_info.reconstruct(&ws, &info);
                            fs::write(&path, dep_info).await?;
                            fs::set_mtime(&path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                // Queue encoded dep-info file (no transformation).
                let path = profile_dir.join(&unit_plan.encoded_dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_compiled_files.encoded_dep_info_file.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_compiled_files.encoded_dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(&path, data).await?;
                            fs::set_mtime(&path, mtime).await?;
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
                    &ws,
                    &mut dep_fingerprints,
                    fingerprint,
                    unit_plan,
                )
                .await?;

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                // Queue all OUT_DIR files with executable flag handling.
                for file in build_script_output_files.out_dir_files {
                    let path: QualifiedPath = serde_json::from_str(file.path.as_str())?;
                    let path = path.reconstruct(&ws, &unit_plan.info).try_into()?;
                    let executable = file.executable;

                    restore_progress
                        .units
                        .get_mut(&unit_hash)
                        .ok_or_eyre("unit hash restore progress not initialized")?
                        .insert(file.object_key.clone());
                    files_to_restore.push(FileRestoreKey {
                        unit_hash: unit_hash.clone(),
                        key: file.object_key.clone(),
                        write: Box::new(move |data| {
                            let data = data.clone();
                            Box::pin(async move {
                                fs::write(&path, data).await?;
                                fs::set_executable(&path, executable).await?;
                                fs::set_mtime(&path, mtime).await?;
                                Ok(())
                            })
                        }),
                    });
                }

                // Queue stdout with BuildScriptOutput reconstruction.
                let ws = ws.clone();
                let info = unit_plan.info.clone();
                let path = profile_dir.join(&unit_plan.stdout_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_output_files.stdout.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_output_files.stdout.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let stdout: cargo::BuildScriptOutput = serde_json::from_slice(&data)?;
                            let stdout = stdout.reconstruct(&ws, &info);
                            fs::write(&path, stdout).await?;
                            fs::set_mtime(&path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                // Queue stderr (no transformation).
                let path = profile_dir.join(&unit_plan.stderr_file()?);
                restore_progress
                    .units
                    .get_mut(&unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_output_files.stderr.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_output_files.stderr.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            fs::write(&path, data).await?;
                            fs::set_mtime(&path, mtime).await?;
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
                fs::set_mtime(&root_output_path, mtime).await?;
            }
            _ => bail!("unit type mismatch"),
        }

        // Mark the unit as restored. It's not _technically_ restored yet, but
        // this function will return an error if the restore doesn't happen
        // anyway.
        restored.record_unit(unit_hash);
    }

    debug!("start sending files to restore workers");
    for file in files_to_restore {
        tx.send_async(file).await?;
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
    restore_progress: RestoreProgress,
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
        restore_batch(
            batch_to_restore,
            &cas,
            &progress,
            &restored,
            &restore_progress,
        )
        .await?;
    }
    debug!("worker rx closed");

    // Once the channel closes, there may still be a partially filled batch
    // remaining. Restore the remaining files in the batch.
    if !batch.is_empty() {
        debug!(?batch, "restoring remaining batch");
        restore_batch(batch, &cas, &progress, &restored, &restore_progress).await?;
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
    restore_progress: &RestoreProgress,
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

                    // Remove the key from the unit's pending keys.
                    let pending_keys = restore_progress
                        .units
                        .get_mut(&file.unit_hash)
                        .ok_or_eyre("unit hash restore progress not initialized")?;
                    // We ignore whether the key is actually present, because
                    // keys might be double-removed if they are present multiple
                    // times in the same unit, which can occur if a unit has two
                    // files that have the same contents (e.g. are both empty).
                    pending_keys.remove(&key);
                    if pending_keys.is_empty() {
                        debug!(?file.unit_hash, "unit has been fully restored");
                        progress.inc(1);
                    }
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
