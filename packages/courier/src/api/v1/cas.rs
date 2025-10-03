use std::sync::LazyLock;

use aerosol::axum::Dep;
use axum::{
    Router,
    body::Body,
    extract::Path,
    http::StatusCode,
    routing::{get, head, put},
};
use hashlru::SyncCache;

use crate::{
    api::State,
    auth::OrgId,
    storage::{Disk, Key},
};

const MAX_KEYS_PER_ORG: usize = 100_000;
const MAX_ORGS: usize = 100;

static ALLOWED_CAS_KEYS: LazyLock<SyncCache<(OrgId, Key), ()>> =
    LazyLock::new(|| SyncCache::new(MAX_ORGS * MAX_KEYS_PER_ORG));

pub fn router() -> Router<State> {
    Router::new()
        .route("/{key}", head(check_cas))
        .route("/{key}", get(read_cas))
        .route("/{key}", put(write_cas))
}

async fn check_cas(Dep(cas): Dep<Disk>, Path(key): Path<String>) -> StatusCode {
    todo!()
}

async fn read_cas(Dep(cas): Dep<Disk>, Path(key): Path<String>) -> Body {
    todo!()
}

async fn write_cas(Dep(cas): Dep<Disk>, Path(key): Path<String>, body: Body) -> StatusCode {
    todo!()
}
