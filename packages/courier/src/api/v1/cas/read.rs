use aerosol::axum::Dep;
use axum::{
    body::Body,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;
use tracing::{error, warn};

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
                // We record access frequency asynchronously to avoid blocking
                // the overall request, since access frequency is a "nice to
                // have" feature while latency is a "must have" feature.
                tokio::spawn(async move {
                    if let Err(err) = db.record_cas_key_access(token.user_id, &key).await {
                        warn!(?err, user = ?token.user_id, "record cas key access");
                    }
                });

                let stream = ReaderStream::new(reader);
                Body::from_stream(stream).into_response()
            }
            Err(err) => {
                error!(?err, "read cas key");
                StatusCode::NOT_FOUND.into_response()
            }
        },
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            error!(?err, "check allowed cas key");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
