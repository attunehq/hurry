use aerosol::axum::Dep;
use axum::{
    body::Body,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::TryStreamExt;
use tokio_util::io::StreamReader;
use tracing::{error, warn};

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

    // Note: [`Disk::write`] validates that the content hashes to the provided
    // key. If the hash doesn't match, the write fails and we return an error
    // without granting database access.
    match cas.write(&key, reader).await {
        Ok(()) => {
            // Grant org access to key in database.
            //
            // We write to disk first, then grant database access. This ordering
            // means that if the database grant fails, we'll have an orphaned
            // blob on disk that no org can access. This is acceptable because:
            // 1. We can't transact across disk and database
            // 2. Writes are idempotent, a retry will succeed
            // 3. Storage is cheaper than blocking writes on database operations
            // 4. Orphaned blobs are a tolerable edge case vs. high write
            //    latency
            // 5. We will likely add a cleanup job for orphaned temp blobs in
            //    the future (reference comments around temp files in
            //    [`Disk::write`]) and we can just clean these up at the same
            //    time.
            if let Err(err) = db.grant_org_cas_key(token.org_id, &key).await {
                error!(?err, user = ?token.user_id, org = ?token.org_id, "error granting org access to cas key");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }

            keysets.organization(token.org_id).insert(key.clone());

            if let Err(err) = db.record_cas_key_access(token.user_id, &key).await {
                warn!(?err, user = ?token.user_id, "error recording cas key access");
            }

            StatusCode::CREATED.into_response()
        }
        Err(err) => {
            error!(?err, "error writing cas key");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
