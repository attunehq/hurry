use axum::{Router, routing::post};
use serde::{Deserialize, Serialize};

use crate::api::State;

pub mod restore;
pub mod save;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactFile {
    pub object_key: String,
    pub path: String,
    pub mtime_nanos: u128,
    pub executable: bool,
}

pub fn router() -> Router<State> {
    Router::new()
        .route("/save", post(save::handle))
        .route("/restore", post(restore::handle))
}
