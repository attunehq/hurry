use axum::{Router, routing::post};

use crate::api::State;

pub mod restore;
pub mod save;

// Re-export shared types
pub use client::courier::v1::cache::{
    ArtifactFile, CargoRestoreRequest, CargoRestoreResponse, CargoSaveRequest,
};

pub fn router() -> Router<State> {
    Router::new()
        .route("/save", post(save::handle))
        .route("/restore", post(restore::handle))
}

impl From<crate::db::CargoArtifact> for ArtifactFile {
    fn from(artifact: crate::db::CargoArtifact) -> Self {
        Self {
            object_key: client::courier::v1::Key::from_hex(&artifact.object_key)
                .expect("database contains valid hex keys"),
            path: artifact.path,
            mtime_nanos: artifact.mtime_nanos,
            executable: artifact.executable,
        }
    }
}

impl From<ArtifactFile> for crate::db::CargoArtifact {
    fn from(artifact: ArtifactFile) -> Self {
        Self {
            object_key: artifact.object_key.to_hex(),
            path: artifact.path,
            mtime_nanos: artifact.mtime_nanos,
            executable: artifact.executable,
        }
    }
}
