use std::collections::HashMap;

use color_eyre::{Result, eyre::bail};
use futures::stream;
use goblin::Object;
use serde::{Deserialize, Serialize};
use tap::TryConv;
use tracing::{debug, instrument, trace, warn};

use crate::{
    cargo::{Restored, RustcTarget, UnitPlan, Workspace},
    cas::CourierCas,
    path::AbsFilePath,
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

    // TODO: This algorithm currently uploads units one at a time. Instead, we
    // should batch units together up to around 10MB in file size for optimal
    // upload speed. One way we could do this is have units present their
    // CAS-able contents, batch those contents up, and then issue save requests
    // for batches of units as their CAS contents are finished uploading.
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
                let files = plan.read(&ws).await?;

                // Prepare CAS objects.
                let mut cas_uploads = Vec::new();

                // TODO: For output files that are `.so` shared objects (e.g.
                // from proc macros or cdylib unit kinds) compiled against
                // glibc, we need to know the glibc version of the imported
                // symbols in the object file.
                let unit_arch = match &plan.info.target_arch {
                    RustcTarget::Specified(target_arch) => target_arch,
                    RustcTarget::ImplicitHost => &ws.host_arch,
                };

                let mut output_files = Vec::new();
                for output_file in files.output_files {
                    let path = output_file
                        .path
                        .clone()
                        .reconstruct(&ws, &plan.info)
                        .try_conv::<AbsFilePath>()?;
                    if unit_arch.uses_glibc()
                        && path
                            .as_std_path()
                            .extension()
                            .is_some_and(|ext| ext == "so")
                    {
                        debug!(?path, "checking glibc version");
                        let object = Object::parse(&output_file.contents)?;
                        match object {
                            Object::Elf(elf) => 'elf: {
                                // Dynamic symbol versions are stored
                                // weirdly[^1]. Each symbol in the dynsyms
                                // section has a corresponding name in the
                                // dynstrtab section.
                                //
                                // Each symbol then also has a corresponding
                                // entry in the versym section. Each versym
                                // entry is a single value, which can be masked
                                // to remove its top bit to get a "file version
                                // identifier" (unless the value is one of the
                                // special values 0 or 1).
                                //
                                // Versions of symbols _imported_ from other
                                // objects are defined in the "Version
                                // Requirements" section, called verneed. Each
                                // verneed entry corresponds to a file, and each
                                // file has multiple "auxiliary" entries that
                                // correspond to specific versions of symbols
                                // imported from that file (this is what you see
                                // when you run `ldd -v` on the object). Each of
                                // these auxiliary entries also has a "file
                                // version identifier" stored in vna_other that
                                // matches up with the identifier in each versym
                                // entry (this is what you see when you run
                                // `objdump -T` on the object).
                                //
                                // [^1]: https://refspecs.linuxbase.org/LSB_5.0.0/LSB-Core-generic/LSB-Core-generic/symversion.html

                                let Some(versym) = elf.versym else {
                                    debug!("no versioned dynamic symbols");
                                    break 'elf;
                                };
                                let Some(verneed) = elf.verneed else {
                                    debug!(
                                        "versioned dynamic symbols are all exports, not imports"
                                    );
                                    break 'elf;
                                };

                                // We start by building a map of file version
                                // identifiers to (file, version) indexes.
                                let mut fvid_to_idx = HashMap::new();
                                for (fidx, need_file) in verneed.iter().enumerate() {
                                    for (vidx, need_ver) in need_file.iter().enumerate() {
                                        fvid_to_idx.insert(need_ver.vna_other, (fidx, vidx));
                                    }
                                }

                                // Now we can iterate through the versym
                                // section, and map each versioned dynamic
                                // symbol to the file and version it needs.
                                let mut symbol_to_fv = HashMap::new();
                                for (sym, versym) in elf.dynsyms.iter().zip(versym.iter()) {
                                    if versym.is_local() || versym.is_global() {
                                        continue;
                                    }

                                    let symbol = elf.dynstrtab.get_at(sym.st_name);
                                    let fvid = versym.version();
                                    let (fidx, vidx) = match fvid_to_idx.get(&fvid) {
                                        Some((fidx, vidx)) => (fidx, vidx),
                                        None => {
                                            warn!("unknown file version identifier: {fvid}");
                                            continue;
                                        }
                                    };
                                    let vn = match verneed.iter().nth(*fidx) {
                                        Some(vn) => vn,
                                        None => {
                                            warn!("unknown Verneed index: {fidx}");
                                            continue;
                                        }
                                    };
                                    let file = elf.dynstrtab.get_at(vn.vn_file);
                                    let vna = match vn.iter().nth(*vidx) {
                                        Some(vna) => vna,
                                        None => {
                                            warn!("unknown Vernaux index: {vidx}");
                                            continue;
                                        }
                                    };
                                    let name = elf.dynstrtab.get_at(vna.vna_name);
                                    symbol_to_fv.insert(symbol, (file, name));
                                }

                                debug!(?symbol_to_fv, "versioned dynamic symbols");
                            }
                            object => {
                                bail!("expected ELF object, got {:?}", object)
                            }
                        }
                    }

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
                        cas_uploads.push((object_key, output_file.contents));
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
                let files = plan.read(&ws).await?;

                // Prepare CAS objects.
                let mut cas_uploads = Vec::new();

                // TODO: For compiled build script programs, we need to know the
                // glibc version of the symbols in the program if we are
                // compiling against glibc.
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
                let files = plan.read(&ws).await?;

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
                        cas_uploads.push((object_key, out_dir_file.contents));
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
