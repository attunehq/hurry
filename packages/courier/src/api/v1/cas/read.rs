use aerosol::axum::Dep;
use axum::{
    body::Body,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;
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
    Dep(cas): Dep<Disk>,
    Dep(db): Dep<Postgres>,
    Path(key): Path<Key>,
) -> Response {
    match check_allowed(&keysets, &db, &key, &token).await {
        Ok(true) => match cas.read(&key).await {
            Ok(reader) => {
                let stream = ReaderStream::new(reader);
                Body::from_stream(stream).into_response()
            }
            Err(err) => {
                error!(?err, "error reading cas key");
                StatusCode::NOT_FOUND.into_response()
            }
        },
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            error!(?err, "error checking allowed cas key");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
