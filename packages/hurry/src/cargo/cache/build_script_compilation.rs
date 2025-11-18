use std::{env::VarError, process::Stdio, time::Duration};

use color_eyre::{Result, Section, SectionExt, eyre::Context as _};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, trace};
use url::Url;
use uuid::Uuid;

use crate::{
    cargo::{
        ArtifactPlan, BuildScriptCompilationUnitPlan, BuildScriptExecutionUnitPlan,
        BuildScriptOutput, DepInfo, Fingerprint, LibraryCrateUnitPlan, QualifiedPath, Workspace,
    },
    cas::CourierCas,
    daemon::{CargoUploadRequest, DaemonPaths},
    progress::TransferBar,
};
use clients::{Courier, Token};

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildScriptCompiledFiles {
    /// This field contains the contents of the compiled build script program at
    /// `build_script_{build_script_entrypoint}-{build_script_compilation_unit_hash}`
    /// and hard linked at `build-script-{build_script_entrypoint}`.
    ///
    /// We need both of these files: the hard link is the file that's actually
    /// executed in the build plan, but the full path with the unit hash is the
    /// file that's tracked by the fingerprint.
    compiled_program: Vec<u8>,
    /// This is the path to the rustc dep-info file in the build directory.
    dep_info_file: DepInfo,
    /// This fingerprint is stored in `.fingerprint`, and is used to derive the
    /// timestamp, fingerprint hash file, and fingerprint JSON file.
    fingerprint: Fingerprint,
    /// This `EncodedDepInfo` (i.e. Cargo dep-info) file is stored in
    /// `.fingerprint`, and is directly saved and restored.
    encoded_dep_info_file: Vec<u8>,
}
