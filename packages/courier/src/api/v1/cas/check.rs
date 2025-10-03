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

pub async fn handle(
    Dep(key_cache): Dep<KeyCache>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<String>,
) -> StatusCode {
    todo!()
}
