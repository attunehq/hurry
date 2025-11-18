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
        cache::SavedFile,
    },
    cas::CourierCas,
    daemon::{CargoUploadRequest, DaemonPaths},
    progress::TransferBar,
};
use clients::{Courier, Token};

// Note that we don't save
// `{profile_dir}/.fingerprint/{package_name}-{unit_hash}/root-output` because
// it is fully reconstructible from the workspace and the unit plan.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildScriptOutputFiles {
    out_dir_files: Vec<SavedFile>,
    stdout: BuildScriptOutput,
    stderr: Vec<u8>,
    fingerprint: Fingerprint,
}
