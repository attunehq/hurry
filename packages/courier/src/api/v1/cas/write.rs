use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode};

use crate::{
    auth::{AuthenticatedStatelessToken, KeySets},
    db::Postgres,
    storage::{Disk, Key},
};

pub async fn handle(
    token: AuthenticatedStatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(cas): Dep<Disk>,
    Dep(db): Dep<Postgres>,
    Path(key): Path<Key>,
    mut body: Body,
) -> StatusCode {
    todo!()
}
