//! GCP-based restore implementation.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime},
};

use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _, bail},
};
use dashmap::{DashMap, DashSet};
use derive_more::Debug;
use futures::{StreamExt, future::BoxFuture};
use tokio::task::JoinSet;
use tracing::{Instrument, debug, info, instrument, trace, warn};

use crate::{
    cargo::{self, Fingerprint, QualifiedPath, UnitHash, UnitPlan, Workspace, host_glibc_version, cache::Restored},
    gcp_cas::GcpCas,
    fs,
    path::JoinWith as _,
    progress::TransferBar,
};
use clients::courier::v1::{Key, SavedUnit};

#[derive(Debug)]
struct FileRestoreKey {
    unit_hash: UnitHash,
    key: Key,
    #[allow(clippy::type_complexity)]
    #[debug(skip)]
    write: Box<dyn FnOnce(&Vec<u8>) -> BoxFuture<'static, Result<()>> + Send + Sync>,
}

#[derive(Debug, Clone, Default)]
struct RestoreProgress {
    units: Arc<DashMap<UnitHash, DashSet<Key>>>,
}

/// Restore units from GCS cache.
#[instrument(skip(units, progress))]
pub async fn restore_units_gcp(
    cas: &GcpCas,
    ws: &Workspace,
    units: &Vec<UnitPlan>,
    progress: &TransferBar,
) -> Result<Restored> {
    trace!(?units, "units");

    let restored = Restored::default();

    // Check which units are already on disk
    let mut units_to_skip: HashSet<UnitHash> = HashSet::new();
    for unit in units {
        let info = unit.info();
        if fs::exists(
            &ws.unit_profile_dir(info)
                .join(unit.fingerprint_json_file()?),
        )
        .await
        {
            units_to_skip.insert(info.unit_hash.clone());
            debug!(
                unit_hash = ?info.unit_hash,
                pkg_name = %info.package_name,
                "skipping unit: already fresh locally"
            );
            restored.units.insert(info.unit_hash.clone());
        }
    }

    // Get host glibc version for filtering
    let host_glibc_symbol_version = host_glibc_version()?;
    debug!(?host_glibc_symbol_version, "restore starting with host glibc");

    // Fetch unit metadata from GCS
    let requested_count = units.len();
    let unit_hashes: Vec<String> = units
        .iter()
        .map(|unit| (&unit.info().unit_hash).into())
        .collect();

    info!(requested_count, "requesting units from GCS cache");
    let mut saved_units = cas
        .restore_units(
            unit_hashes,
            host_glibc_symbol_version.as_ref().map(|v| v.to_string()).as_deref(),
        )
        .await?;
    info!(
        requested_count,
        returned_count = saved_units.len(),
        "GCS cache restore response"
    );

    // Track restore progress
    let restore_progress = RestoreProgress::default();

    // Spawn concurrent workers for parallel downloads
    let (tx, mut workers) = {
        let worker_count = num_cpus::get();
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
        (tx, workers)
    };

    let mut dep_fingerprints = HashMap::new();
    let mut files_to_restore = Vec::<FileRestoreKey>::new();
    let starting_mtime = SystemTime::UNIX_EPOCH;
    let ws = Arc::new(ws.clone());

    for (i, unit) in units.iter().enumerate() {
        debug!(?unit, "queuing unit restore");
        let unit_hash = &unit.info().unit_hash;
        let mtime = starting_mtime + Duration::from_secs(i as u64);

        let Some(saved) = saved_units.take(&unit_hash.into()) else {
            debug!(
                ?unit_hash,
                pkg_name = %unit.info().package_name,
                "unit missing from cache response"
            );

            if units_to_skip.contains(unit_hash)
                && let Err(err) = unit.touch(&ws, starting_mtime).await
            {
                warn!(?unit_hash, ?err, "could not set mtime for skipped unit");
            }
            progress.dec_length(1);
            continue;
        };

        let cached_fingerprint = saved.fingerprint().as_str();
        let cached_fingerprint = serde_json::from_str::<Fingerprint>(cached_fingerprint)?;

        if units_to_skip.contains(unit_hash) {
            let profile = ws.unit_profile_dir(unit.info());
            let cached_hash = cached_fingerprint.hash_u64();

            let file = unit.fingerprint_json_file()?;
            let file = profile.join(&file);
            let json = fs::must_read_buffered_utf8(&file).await?;
            let local = serde_json::from_str::<Fingerprint>(&json)?;
            let local_hash = local.hash_u64();

            debug!(
                ?cached_hash,
                ?local_hash,
                "recorded fingerprint mapping for skipped unit"
            );

            dep_fingerprints.insert(cached_hash, local);

            if let Err(err) = unit.touch(&ws, mtime).await {
                warn!(?unit_hash, ?err, "could not set mtime for skipped unit");
            }
            progress.dec_length(1);
            continue;
        }

        let info = unit.info();
        let src_path = unit.src_path().map(|p| p.into());
        let rewritten_fingerprint = cached_fingerprint.rewrite(src_path, &mut dep_fingerprints)?;
        let fingerprint_hash = rewritten_fingerprint.fingerprint_hash();

        let profile_dir = ws.unit_profile_dir(info);
        fs::write(
            &profile_dir.join(&unit.fingerprint_hash_file()?),
            fingerprint_hash,
        )
        .await?;
        fs::write(
            &profile_dir.join(&unit.fingerprint_json_file()?),
            serde_json::to_vec(&rewritten_fingerprint)?,
        )
        .await?;

        restore_progress
            .units
            .insert(unit_hash.clone(), DashSet::new());

        // Queue files for restoration based on unit type
        match (saved, unit) {
            (
                SavedUnit::LibraryCrate(saved_library_files, _),
                UnitPlan::LibraryCrate(unit_plan),
            ) => {
                trace!(
                    pkg_name = %unit_plan.info.package_name,
                    unit_hash = %unit_plan.info.unit_hash,
                    "restoring library crate unit"
                );

                for file in saved_library_files.output_files {
                    let path: QualifiedPath = serde_json::from_str(file.path.as_str())?;
                    let path = path.reconstruct(&ws, &unit_plan.info).try_into()?;
                    let executable = file.executable;

                    restore_progress
                        .units
                        .get_mut(unit_hash)
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

                let ws_clone = ws.clone();
                let info = unit_plan.info.clone();
                let path = profile_dir.join(&unit_plan.dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(saved_library_files.dep_info_file.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: saved_library_files.dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let dep_info: cargo::DepInfo = serde_json::from_slice(&data)?;
                            let dep_info = dep_info.reconstruct(&ws_clone, &info);
                            fs::write(&path, dep_info).await?;
                            fs::set_mtime(&path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                let path = profile_dir.join(&unit_plan.encoded_dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
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
                debug!(
                    pkg_name = %unit_plan.info.package_name,
                    unit_hash = %unit_plan.info.unit_hash,
                    "restoring build script compilation unit"
                );

                let profile_dir = ws.unit_profile_dir(&unit_plan.info);

                let path = profile_dir.join(unit_plan.program_file()?);
                let linked_path = profile_dir.join(unit_plan.linked_program_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
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

                let ws_clone = ws.clone();
                let info = unit_plan.info.clone();
                let path = profile_dir.join(&unit_plan.dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_compiled_files.dep_info_file.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_compiled_files.dep_info_file.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let dep_info: cargo::DepInfo = serde_json::from_slice(&data)?;
                            let dep_info = dep_info.reconstruct(&ws_clone, &info);
                            fs::write(&path, dep_info).await?;
                            fs::set_mtime(&path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                let path = profile_dir.join(&unit_plan.encoded_dep_info_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
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
                let profile_dir = ws.unit_profile_dir(&unit_plan.info);
                let out_dir = unit_plan.out_dir()?;
                let out_dir_absolute = profile_dir.join(&out_dir);

                debug!(
                    pkg_name = %unit_plan.info.package_name,
                    unit_hash = %unit_plan.info.unit_hash,
                    out_dir = %out_dir,
                    "restoring build script execution unit"
                );

                fs::create_dir_all(&out_dir_absolute).await?;

                for file in build_script_output_files.out_dir_files {
                    let path: QualifiedPath = serde_json::from_str(file.path.as_str())?;
                    let path = path.reconstruct(&ws, &unit_plan.info).try_into()?;
                    let executable = file.executable;

                    restore_progress
                        .units
                        .get_mut(unit_hash)
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

                let ws_clone = ws.clone();
                let info = unit_plan.info.clone();
                let path = profile_dir.join(&unit_plan.stdout_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
                    .ok_or_eyre("unit hash restore progress not initialized")?
                    .insert(build_script_output_files.stdout.clone());
                files_to_restore.push(FileRestoreKey {
                    unit_hash: unit_hash.clone(),
                    key: build_script_output_files.stdout.clone(),
                    write: Box::new(move |data| {
                        let data = data.clone();
                        Box::pin(async move {
                            let stdout: cargo::BuildScriptOutput = serde_json::from_slice(&data)?;
                            let stdout = stdout.reconstruct(&ws_clone, &info);
                            fs::write(&path, stdout).await?;
                            fs::set_mtime(&path, mtime).await?;
                            Ok(())
                        })
                    }),
                });

                let path = profile_dir.join(&unit_plan.stderr_file()?);
                restore_progress
                    .units
                    .get_mut(unit_hash)
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

                let root_output_path = profile_dir.join(&unit_plan.root_output_file()?);
                fs::write(
                    &root_output_path,
                    out_dir_absolute.as_os_str().as_encoded_bytes(),
                )
                .await?;
                fs::set_mtime(&root_output_path, mtime).await?;
            }
            _ => bail!("unit type mismatch"),
        }

        debug!(?unit, "marking unit as restored");
        restored.units.insert(unit_hash.clone());
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
    cas: GcpCas,
    progress: TransferBar,
    restored: Restored,
    restore_progress: RestoreProgress,
) -> Result<()> {
    const BATCH_SIZE: usize = 50;
    let mut batch = Vec::new();
    while let Ok(file) = rx.recv_async().await {
        debug!(?file, "worker got file");
        batch.push(file);
        if batch.len() < BATCH_SIZE {
            continue;
        }

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

    if !batch.is_empty() {
        restore_batch(batch, &cas, &progress, &restored, &restore_progress).await?;
    }

    Ok(())
}

#[instrument(skip_all)]
async fn restore_batch(
    batch: Vec<FileRestoreKey>,
    cas: &GcpCas,
    progress: &TransferBar,
    restored: &Restored,
    restore_progress: &RestoreProgress,
) -> Result<()> {
    debug!(?batch, "restoring batch");

    let mut key_to_files = HashMap::new();
    for file in batch {
        key_to_files
            .entry(file.key.clone())
            .or_insert(vec![])
            .push(file);
    }

    let keys = key_to_files.keys().cloned().collect::<Vec<_>>();

    debug!(?keys, "start fetching files from GCS");
    let mut res = cas.get_bulk(keys).await?;
    debug!("start streaming response from GCS");
    while let Some(result) = res.next().await {
        match result {
            Ok((key, data)) => {
                debug!(?key, "GCS stream entry");
                let files = key_to_files
                    .remove(&key)
                    .ok_or_eyre("unrecognized key from GCS bulk response")?;
                for file in files {
                    restored.files.insert(file.key);

                    progress.add_files(1);
                    progress.add_bytes(data.len() as u64);

                    debug!(?key, "calling write callback");
                    (file.write)(&data).await?;
                    debug!(?key, "done calling write callback");

                    let pending_keys = restore_progress
                        .units
                        .get_mut(&file.unit_hash)
                        .ok_or_eyre("unit hash restore progress not initialized")?;
                    pending_keys.remove(&key);
                    if pending_keys.is_empty() {
                        debug!(?file.unit_hash, "unit has been fully restored");
                        progress.inc(1);
                    }
                }
            }
            Err(error) => {
                warn!(?error, "failed to fetch file from GCS");
            }
        }
    }
    debug!("done streaming response from GCS");

    Ok(())
}
