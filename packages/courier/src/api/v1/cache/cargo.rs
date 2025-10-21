use axum::{Router, routing::post};
use client::courier::v1::{Key, cache::ArtifactFile};

use crate::api::State;

pub mod restore;
pub mod save;

pub fn router() -> Router<State> {
    Router::new()
        .route("/save", post(save::handle))
        .route("/restore", post(restore::handle))
}

impl From<crate::db::CargoArtifact> for ArtifactFile {
    fn from(artifact: crate::db::CargoArtifact) -> Self {
        ArtifactFile::builder()
            .object_key(
                Key::from_hex(&artifact.object_key).expect("database contains valid hex keys"),
            )
            .executable(artifact.executable)
            .mtime_nanos(artifact.mtime_nanos)
            .path(artifact.path)
            .build()
    }
}

impl From<ArtifactFile> for crate::db::CargoArtifact {
    fn from(artifact: ArtifactFile) -> Self {
        Self {
            object_key: artifact.object_key.to_hex(),
            path: String::from(artifact.path.as_str()),
            mtime_nanos: artifact.mtime_nanos,
            executable: artifact.executable,
        }
    }
}
