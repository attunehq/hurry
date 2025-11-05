use serde::{Deserialize, Serialize};

use clients::courier::v1::Key;
use derive_more::Debug;
use url::Url;
use uuid::Uuid;

use crate::cargo::{ArtifactKey, ArtifactPlan, SaveProgress, Workspace};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadRequest {
    pub request_id: Uuid,
    pub courier_url: Url,
    pub ws: Workspace,
    #[debug(skip)]
    pub artifact_plan: ArtifactPlan,
    pub skip_artifacts: Vec<ArtifactKey>,
    pub skip_objects: Vec<Key>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CargoUploadStatus {
    InProgress(SaveProgress),
    Complete,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadStatusRequest {
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CargoUploadStatusResponse {
    pub status: Option<CargoUploadStatus>,
}
