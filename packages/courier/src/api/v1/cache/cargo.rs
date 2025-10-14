use axum::{Router, routing::post};
use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::api::State;

pub mod restore;
pub mod save;

pub fn router() -> Router<State> {
    Router::new()
        .route("/save", post(save::handle))
        .route("/restore", post(restore::handle))
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct ArtifactFile {
    pub object_key: String,
    pub path: String,
    pub mtime_nanos: u128,
    pub executable: bool,
}

impl From<crate::db::CargoArtifact> for ArtifactFile {
    fn from(artifact: crate::db::CargoArtifact) -> Self {
        Self {
            object_key: artifact.object_key,
            path: artifact.path,
            mtime_nanos: artifact.mtime_nanos,
            executable: artifact.executable,
        }
    }
}

impl From<ArtifactFile> for crate::db::CargoArtifact {
    fn from(artifact: ArtifactFile) -> Self {
        Self {
            object_key: artifact.object_key,
            path: artifact.path,
            mtime_nanos: artifact.mtime_nanos,
            executable: artifact.executable,
        }
    }
}
