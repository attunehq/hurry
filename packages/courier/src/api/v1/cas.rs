use aerosol::axum::Dep;
use axum::{
    Router,
    body::Body,
    extract::Path,
    http::StatusCode,
    routing::{get, head, put},
};

use crate::{
    api::State,
    auth::{KeyCache, OrgId},
    storage::{Disk, Key},
};

pub mod check;
pub mod read;
pub mod write;

pub fn router() -> Router<State> {
    Router::new()
        .route("/{key}", head(check::handle))
        .route("/{key}", get(read::handle))
        .route("/{key}", put(write::handle))
}
