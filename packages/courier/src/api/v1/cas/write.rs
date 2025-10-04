use aerosol::axum::Dep;
use axum::{
    body::Body,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::TryStreamExt;
use tokio_util::io::StreamReader;
use tracing::error;

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
    body: Body,
) -> Response {
    let stream = body.into_data_stream();
    let stream = stream.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err));
    let reader = StreamReader::new(stream);

    match cas.write(&key, reader).await {
        Ok(()) => {
            // Grant org access to key in database
            if let Err(err) = db.grant_org_cas_key(token.org_id, &key).await {
                error!(?err, "error granting org access to cas key");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }

            // Add key to in-memory cache for this org
            keysets.organization(token.org_id).insert(key.clone());

            // Asynchronously record access frequency
            let db = db.clone();
            let user_id = token.user_id;
            let key = key.clone();
            tokio::spawn(async move {
                if let Err(err) = db.record_cas_key_access(user_id, &key).await {
                    error!(?err, "error recording cas key access");
                }
            });

            StatusCode::CREATED.into_response()
        }
        Err(err) => {
            error!(?err, "error writing cas key");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
