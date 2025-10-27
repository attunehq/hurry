use serde::{Deserialize, Serialize};

use crate::cargo::{ArtifactKey, ArtifactPlan, Workspace};
use clients::courier::v1::Key;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoUploadRequest {
    pub ws: Workspace,
    pub artifact_plan: ArtifactPlan,
    pub skip_artifacts: Vec<ArtifactKey>,
    pub skip_objects: Vec<Key>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoUploadResponse {
    pub ok: bool,
}
