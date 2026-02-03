//! GCP-based save implementation.

use std::{collections::HashMap, path::PathBuf};

use color_eyre::{Result, eyre::bail};
use futures::stream;
use tap::{Conv as _, Pipe as _};
use tracing::{debug, error, instrument, trace};

use crate::{
    cargo::{
        Fingerprint, QualifiedPath, RustcTarget, UnitPlan, Workspace, host_glibc_version,
        cache::{Restored, SaveProgress},
    },
    gcp_cas::GcpCas,
    path::{AbsDirPath, AbsFilePath, JoinWith as _},
};
use clients::courier::v1::{self as courier, Key};

/// Save units to GCS cache.
#[instrument(skip_all)]
pub async fn save_units_gcp(
    cas: &GcpCas,
    ws: Workspace,
    units: Vec<UnitPlan>,
    skip: Restored,
    mut on_progress: impl FnMut(&SaveProgress),
) -> Result<()> {
    trace!(?units, ?skip, "saving units to GCS");

    let mut progress = SaveProgress {
        uploaded_units: 0,
        total_units: units.len() as u64,
        uploaded_files: 0,
        uploaded_bytes: 0,
    };

    let mut dep_fingerprints = HashMap::new();
    for unit in units {
        debug!(?unit, "saving unit");
        if skip.units.contains(&unit.info().unit_hash) {
            debug!(?unit, "skipping unit backup: unit was restored from cache");
            progress.total_units -= 1;
            on_progress(&progress);

            rewrite_fingerprint(
                &ws,
                &unit.info().target_arch,
                unit.src_path(),
                &mut dep_fingerprints,
                unit.read_fingerprint(&ws).await?,
            )
            .await?;

            continue;
        }

        let unit_arch = match &unit.info().target_arch {
            RustcTarget::Specified(target_arch) => &target_arch.clone(),
            RustcTarget::ImplicitHost => &ws.host_arch,
        };
        let glibc_version = if unit_arch.uses_glibc() {
            if unit_arch != &ws.host_arch {
                error!("backing up cross-compiled units is not yet supported");
                progress.total_units -= 1;
                on_progress(&progress);
                continue;
            }
            host_glibc_version()?
        } else {
            None
        };

        let unit_hash: String = (&unit.info().unit_hash).into();

        match unit {
            UnitPlan::LibraryCrate(plan) => {
                let files = plan.read(&ws).await?;

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

                if !cas_uploads.is_empty() {
                    cas.store_bulk(stream::iter(cas_uploads)).await?;
                }

                let fingerprint = rewrite_fingerprint(
                    &ws,
                    &plan.info.target_arch,
                    Some(plan.src_path.clone()),
                    &mut dep_fingerprints,
                    files.fingerprint,
                )
                .await?;

                let saved_unit = courier::SavedUnit::LibraryCrate(
                    courier::LibraryFiles::builder()
                        .output_files(output_files)
                        .dep_info_file(dep_info_file)
                        .encoded_dep_info_file(encoded_dep_info_file)
                        .fingerprint(fingerprint)
                        .build(),
                    plan.try_into()?,
                );

                cas.save_unit(
                    &unit_hash,
                    &saved_unit,
                    unit_arch.as_str(),
                    glibc_version.as_ref().map(|v| v.to_string()).as_deref(),
                )
                .await?;
            }
            UnitPlan::BuildScriptCompilation(plan) => {
                let files = plan.read(&ws).await?;

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

                if !cas_uploads.is_empty() {
                    cas.store_bulk(stream::iter(cas_uploads)).await?;
                }

                let fingerprint = rewrite_fingerprint(
                    &ws,
                    &plan.info.target_arch,
                    Some(plan.src_path.clone()),
                    &mut dep_fingerprints,
                    files.fingerprint,
                )
                .await?;

                let saved_unit = courier::SavedUnit::BuildScriptCompilation(
                    courier::BuildScriptCompiledFiles::builder()
                        .compiled_program(compiled_program)
                        .dep_info_file(dep_info_file)
                        .fingerprint(fingerprint)
                        .encoded_dep_info_file(encoded_dep_info_file)
                        .build(),
                    plan.try_into()?,
                );

                cas.save_unit(
                    &unit_hash,
                    &saved_unit,
                    unit_arch.as_str(),
                    glibc_version.as_ref().map(|v| v.to_string()).as_deref(),
                )
                .await?;
            }
            UnitPlan::BuildScriptExecution(plan) => {
                let files = plan.read(&ws).await?;

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

                if !cas_uploads.is_empty() {
                    cas.store_bulk(stream::iter(cas_uploads)).await?;
                }

                let fingerprint = rewrite_fingerprint(
                    &ws,
                    &plan.info.target_arch,
                    None,
                    &mut dep_fingerprints,
                    files.fingerprint,
                )
                .await?;

                let saved_unit = courier::SavedUnit::BuildScriptExecution(
                    courier::BuildScriptOutputFiles::builder()
                        .out_dir_files(out_dir_files)
                        .stdout(stdout)
                        .stderr(stderr)
                        .fingerprint(fingerprint)
                        .build(),
                    plan.try_into()?,
                );

                cas.save_unit(
                    &unit_hash,
                    &saved_unit,
                    unit_arch.as_str(),
                    glibc_version.as_ref().map(|v| v.to_string()).as_deref(),
                )
                .await?;
            }
        }
        progress.uploaded_units += 1;
        on_progress(&progress);
    }

    Result::<_>::Ok(())
}

#[instrument(skip_all)]
async fn rewrite_fingerprint(
    ws: &Workspace,
    target: &RustcTarget,
    src_path: Option<AbsFilePath>,
    dep_fingerprints: &mut HashMap<u64, Fingerprint>,
    fingerprint: Fingerprint,
) -> Result<courier::Fingerprint> {
    let src_path = match src_path {
        Some(ref src_path) => {
            let qualified = QualifiedPath::parse_abs(ws, target, src_path);
            match qualified {
                QualifiedPath::Rootless(p) => {
                    bail!("impossible: fingerprint path is not absolute: {}", p)
                }
                QualifiedPath::RelativeTargetProfile(p) => {
                    bail!("unexpected fingerprint path root: {}", p)
                }
                QualifiedPath::Absolute(p) => bail!("unexpected fingerprint path root: {}", p),
                QualifiedPath::RelativeCargoHome(p) => AbsDirPath::try_from("/cargo_home")?
                    .join(p)
                    .conv::<PathBuf>()
                    .pipe(Some),
            }
        }
        None => None,
    };
    let rewritten_fingerprint = fingerprint.rewrite(src_path, dep_fingerprints)?;
    serde_json::to_string(&rewritten_fingerprint)?
        .conv::<courier::Fingerprint>()
        .pipe(Ok)
}
