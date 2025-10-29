use serde::{Deserialize, Serialize};

use clients::courier::v1::Key;
use url::Url;

use crate::cargo::{ArtifactKey, ArtifactPlan, Workspace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoUploadRequest {
    pub courier_url: Url,
    pub ws: Workspace,
    pub artifact_plan: ArtifactPlan,
    pub skip_artifacts: Vec<ArtifactKey>,
    pub skip_objects: Vec<Key>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoUploadResponse {
    pub ok: bool,
}
