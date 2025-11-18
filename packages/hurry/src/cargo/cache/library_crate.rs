use std::{env::VarError, process::Stdio, time::Duration};

use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context as _, bail},
};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, trace};
use url::Url;
use uuid::Uuid;

use crate::{
    cargo::{
        ArtifactPlan, BuildScriptCompilationUnitPlan, BuildScriptExecutionUnitPlan,
        BuildScriptOutput, DepInfo, Fingerprint, LibraryCrateUnitPlan, QualifiedPath, Workspace,
        cache::SavedFile,
    },
    cas::CourierCas,
    daemon::{CargoUploadRequest, DaemonPaths},
    fs,
    path::JoinWith as _,
    progress::TransferBar,
};
use clients::{Courier, Token};

/// Libraries are usually associated with 7 files:
///
/// - 2 output files (an `.rmeta` and an `.rlib`)
/// - 1 rustc dep-info (`.d`) file in the `deps` folder
/// - 4 files in the fingerprint directory
///   - An `EncodedDepInfo` file
///   - A fingerprint hash
///   - A fingerprint JSON
///   - An invoked timestamp
///
/// Of these files, the fingerprint hash, fingerprint JSON, and invoked
/// timestamp are all reconstructed from fingerprint information during
/// restoration.
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryFiles {
    /// These files come from the build plan's `outputs` field.
    // TODO: Can we specify this even more narrowly (e.g. with an `rmeta` and
    // `rlib` field)? I know there are other possible output files (e.g. `.so`
    // for proc macros on Linux and `.dylib` for something on macOS), but I
    // don't know what the enumerated list is.
    output_files: Vec<SavedFile>,
    /// This file is always at a known path in
    /// `deps/{package_name}-{unit_hash}.d`.
    dep_info_file: DepInfo,
    /// This information is parsed from the initial fingerprint created after
    /// the build, and is used to dynamically reconstruct fingerprints on
    /// restoration.
    fingerprint: Fingerprint,
    /// This file is always at a known path in
    /// `.fingerprint/{package_name}-{unit_hash}/dep-lib-{crate_name}`. It can
    /// be safely relocatably copied because the `EncodedDepInfo` struct only
    /// ever contains relative file path information (note that deps always have
    /// a `DepInfoPathType`, which is either `PackageRootRelative` or
    /// `BuildRootRelative`)[^1].
    ///
    /// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/dep_info.rs#L112
    encoded_dep_info_file: Vec<u8>,
}

impl LibraryFiles {
    async fn from_plan(workspace: &Workspace, lib_unit: LibraryCrateUnitPlan) -> Result<Self> {
        let outputs = &lib_unit.outputs;
        let unit_info = &lib_unit.info;
        let profile_dir = workspace.unit_profile_dir(&unit_info)?;

        let output_files = {
            let mut output_files = Vec::new();
            for output_file_path in outputs.into_iter() {
                let path = QualifiedPath::parse(
                    &workspace,
                    &unit_info.target_arch,
                    &output_file_path.clone().into(),
                )
                .await?;
                let contents = fs::must_read_buffered(&output_file_path).await?;
                let executable = fs::is_executable(&output_file_path.as_std_path()).await;
                output_files.push(SavedFile {
                    path,
                    contents,
                    executable,
                });
            }
            output_files
        };

        let dep_info_file = DepInfo::from_file(
            &workspace,
            &unit_info.target_arch,
            &profile_dir.join(&lib_unit.dep_info_file()?),
        )
        .await?;

        let encoded_dep_info_file =
            fs::must_read_buffered(&profile_dir.join(&lib_unit.encoded_dep_info_file()?)).await?;

        let fingerprint = {
            let fingerprint_json =
                fs::must_read_buffered_utf8(&profile_dir.join(&lib_unit.fingerprint_json_file()?))
                    .await?;
            let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

            let fingerprint_hash =
                fs::must_read_buffered_utf8(&profile_dir.join(&lib_unit.fingerprint_hash_file()?))
                    .await?;

            // Sanity check that the fingerprint hashes match.
            if fingerprint.fingerprint_hash() != fingerprint_hash {
                bail!("fingerprint hash mismatch");
            }

            fingerprint
        };

        Ok(Self {
            output_files,
            dep_info_file,
            fingerprint,
            encoded_dep_info_file,
        })
    }
}
