use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tokio_util::io::ReaderStream;
use tracing::{error, warn};

use crate::{
    api::v1::cas::check_allowed,
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::{Disk, Key},
};

/// Read the content from the CAS for the given key.
///
/// ## Security
///
/// All users have visibility into all keys that any user in the organization
/// has ever written. This is intentional, because we expect users to run hurry
/// on their local dev machines as well as in CI or other environments like
/// docker builds.
///
/// Even if another organization has written content with the same key, this
/// content is not visible to the current organization unless they have also
/// written it.
pub async fn handle(
    token: StatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(cas): Dep<Disk>,
    Dep(db): Dep<Postgres>,
    Path(key): Path<Key>,
) -> CasReadResponse {
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
                CasReadResponse::Found(Body::from_stream(stream))
            }
            Err(err) => {
                error!(?err, "read cas key");
                CasReadResponse::Error(err)
            }
        },
        Ok(false) => CasReadResponse::NotFound,
        Err(err) => {
            error!(?err, "check allowed cas key");
            CasReadResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CasReadResponse {
    Found(Body),
    NotFound,
    Error(Report),
}

impl IntoResponse for CasReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasReadResponse::Found(body) => (StatusCode::OK, body).into_response(),
            CasReadResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CasReadResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
