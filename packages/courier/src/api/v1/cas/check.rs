use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode};
use tracing::error;

use crate::{
    api::v1::cas::check_allowed,
    auth::{AuthenticatedStatelessToken, KeySets},
    db::Postgres,
    storage::{Disk, Key},
};

pub async fn handle(
    token: AuthenticatedStatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
) -> StatusCode {
    match check_allowed(&keysets, &db, &key, &token).await {
        Ok(true) => {
            if cas.exists(&key).await {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        }
        Ok(false) => {
            StatusCode::NOT_FOUND
        }
        Err(err) => {
            error!(?err, "error checking allowed cas key");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
