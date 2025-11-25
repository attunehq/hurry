use color_eyre::Result;
use futures::stream;
use serde::{Deserialize, Serialize};
use tracing::{instrument, trace};

use crate::{
    cargo::{Restored, UnitPlan, Workspace, cache},
    cas::CourierCas,
};
use clients::{
    Courier,
    courier::v1::{
        self as courier, Key,
        cache::{CargoSaveRequest, CargoSaveUnitRequest, SavedUnitCacheKey},
    },
};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SaveProgress {
    pub uploaded_units: u64,
    pub total_units: u64,
    pub uploaded_files: u64,
    pub uploaded_bytes: u64,
}

#[instrument(skip_all)]
pub async fn save_units(
    courier: &Courier,
    cas: &CourierCas,
    ws: Workspace,
    units: Vec<UnitPlan>,
    skip: Restored,
    mut on_progress: impl FnMut(&SaveProgress),
) -> Result<()> {
    trace!(?units, "units");

    let mut progress = SaveProgress {
        uploaded_units: 0,
        total_units: units.len() as u64,
        uploaded_files: 0,
        uploaded_bytes: 0,
    };

    // TODO: Batch units together up to around 10MB in file size for optimal
    // upload speed. One way we could do this is have units present their
    // CAS-able contents, batch those contents up, and then issue save requests
    // for batches of units as their CAS contents are finished uploading.

    // This algorithm currently uploads units one at a time, and only skips uploads
    // at the unit level (not at the file level).
    //
    // TODO: Skip uploads at the file object level.
    let mut save_requests = Vec::new();
    for unit in units {
        if skip.units.contains(&unit.info().unit_hash) {
            trace!(?unit, "skipping unit backup: unit was restored from cache");
            progress.total_units -= 1;
            on_progress(&progress);
            continue;
        }

        // Upload unit to CAS and cache.
        match unit {
            UnitPlan::LibraryCrate(plan) => {
                // Read unit files.
                let files = cache::LibraryFiles::read(&ws, &plan).await?;

                // Prepare CAS objects.
                let mut cas_uploads = Vec::new();

                let mut output_files = Vec::new();
                for output_file in files.output_files {
                    let object_key = Key::from_buffer(&output_file.contents);
                    output_files.push(
                        courier::SavedFile::builder()
                            .object_key(object_key.clone())
                            .executable(output_file.executable)
                            .path(serde_json::to_string(&output_file.path)?)
                            .build(),
                    );

                    if !skip.files.contains(&object_key) {
                        progress.uploaded_files += 1;
                        progress.uploaded_bytes += output_file.contents.len() as u64;
                        cas_uploads.push((object_key.clone(), output_file.contents));
                    }
                }

                let dep_info_file_contents = serde_json::to_vec(&files.dep_info_file)?;
                let dep_info_file = Key::from_buffer(&dep_info_file_contents);
                if !skip.files.contains(&dep_info_file) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += dep_info_file_contents.len() as u64;
                    cas_uploads.push((dep_info_file.clone(), dep_info_file_contents));
                }

                let encoded_dep_info_file = Key::from_buffer(&files.encoded_dep_info_file);
                if !skip.files.contains(&encoded_dep_info_file) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += files.encoded_dep_info_file.len() as u64;
                    cas_uploads.push((encoded_dep_info_file.clone(), files.encoded_dep_info_file));
                }

                // Save CAS objects.
                if !cas_uploads.is_empty() {
                    cas.store_bulk(stream::iter(cas_uploads)).await?;
                }

                // Prepare save request.
                let fingerprint = serde_json::to_string(&files.fingerprint)?;
                let save_request = CargoSaveUnitRequest::builder()
                    .key(
                        SavedUnitCacheKey::builder()
                            .unit_hash(plan.info.clone().unit_hash)
                            .build(),
                    )
                    .unit(courier::SavedUnit::LibraryCrate(
                        courier::LibraryFiles::builder()
                            .output_files(output_files)
                            .dep_info_file(dep_info_file)
                            .encoded_dep_info_file(encoded_dep_info_file)
                            .fingerprint(fingerprint.into())
                            .build(),
                        plan.try_into()?,
                    ))
                    .build();

                save_requests.push(save_request);
            }
            UnitPlan::BuildScriptCompilation(plan) => {
                // Read unit files.
                let files = cache::BuildScriptCompiledFiles::read(&ws, &plan).await?;

                // Prepare CAS objects.
                let mut cas_uploads = Vec::new();

                let compiled_program = Key::from_buffer(&files.compiled_program);
                if !skip.files.contains(&compiled_program) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += files.compiled_program.len() as u64;
                    cas_uploads.push((compiled_program.clone(), files.compiled_program));
                }

                let dep_info_file_contents = serde_json::to_vec(&files.dep_info_file)?;
                let dep_info_file = Key::from_buffer(&dep_info_file_contents);
                if !skip.files.contains(&dep_info_file) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += dep_info_file_contents.len() as u64;
                    cas_uploads.push((dep_info_file.clone(), dep_info_file_contents));
                }

                let encoded_dep_info_file = Key::from_buffer(&files.encoded_dep_info_file);
                if !skip.files.contains(&encoded_dep_info_file) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += files.encoded_dep_info_file.len() as u64;
                    cas_uploads.push((encoded_dep_info_file.clone(), files.encoded_dep_info_file));
                }

                // Save CAS objects.
                if !cas_uploads.is_empty() {
                    cas.store_bulk(stream::iter(cas_uploads)).await?;
                }

                // Prepare save request.
                let fingerprint = serde_json::to_string(&files.fingerprint)?;
                let save_request = CargoSaveUnitRequest::builder()
                    .key(
                        SavedUnitCacheKey::builder()
                            .unit_hash(plan.info.clone().unit_hash)
                            .build(),
                    )
                    .unit(courier::SavedUnit::BuildScriptCompilation(
                        courier::BuildScriptCompiledFiles::builder()
                            .compiled_program(compiled_program)
                            .dep_info_file(dep_info_file)
                            .fingerprint(fingerprint)
                            .encoded_dep_info_file(encoded_dep_info_file)
                            .build(),
                        plan.try_into()?,
                    ))
                    .build();

                save_requests.push(save_request);
            }
            UnitPlan::BuildScriptExecution(plan) => {
                // Read unit files.
                let files = cache::BuildScriptOutputFiles::read(&ws, &plan).await?;

                // Prepare CAS objects.
                let mut cas_uploads = Vec::new();

                let mut out_dir_files = Vec::new();
                for out_dir_file in files.out_dir_files {
                    let object_key = Key::from_buffer(&out_dir_file.contents);
                    out_dir_files.push(
                        courier::SavedFile::builder()
                            .object_key(object_key.clone())
                            .executable(out_dir_file.executable)
                            .path(serde_json::to_string(&out_dir_file.path)?)
                            .build(),
                    );

                    if !skip.files.contains(&object_key) {
                        progress.uploaded_files += 1;
                        progress.uploaded_bytes += out_dir_file.contents.len() as u64;
                        cas_uploads.push((object_key.clone(), out_dir_file.contents));
                    }
                }

                let stdout_contents = serde_json::to_vec(&files.stdout)?;
                let stdout = Key::from_buffer(&stdout_contents);
                if !skip.files.contains(&stdout) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += stdout_contents.len() as u64;
                    cas_uploads.push((stdout.clone(), stdout_contents));
                }

                let stderr = Key::from_buffer(&files.stderr);
                if !skip.files.contains(&stderr) {
                    progress.uploaded_files += 1;
                    progress.uploaded_bytes += files.stderr.len() as u64;
                    cas_uploads.push((stderr.clone(), files.stderr));
                }

                // Save CAS objects.
                if !cas_uploads.is_empty() {
                    cas.store_bulk(stream::iter(cas_uploads)).await?;
                }

                // Prepare save request.
                let fingerprint = serde_json::to_string(&files.fingerprint)?;
                let save_request = CargoSaveUnitRequest::builder()
                    .key(
                        SavedUnitCacheKey::builder()
                            .unit_hash(plan.info.clone().unit_hash)
                            .build(),
                    )
                    .unit(courier::SavedUnit::BuildScriptExecution(
                        courier::BuildScriptOutputFiles::builder()
                            .out_dir_files(out_dir_files)
                            .stdout(stdout)
                            .stderr(stderr)
                            .fingerprint(fingerprint)
                            .build(),
                        plan.try_into()?,
                    ))
                    .build();

                save_requests.push(save_request);
            }
        }
        progress.uploaded_units += 1;
        on_progress(&progress);
    }

    // Save units to remote cache.
    courier
        .cargo_cache_save(CargoSaveRequest::new(save_requests))
        .await?;

    Result::<_>::Ok(())
}
