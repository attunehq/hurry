use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use futures::TryStreamExt;
use tokio_util::io::StreamReader;
use tracing::{error, warn};

use crate::{
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::{Disk, Key},
};

/// Write the content to the CAS for the given key.
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
///
/// ## Idempotency
///
/// The CAS is idempotent: if a file already exists, it is not written again.
/// This is safe because the key is computed from the content of the file, so if
/// the file already exists it must have the same content.
///
/// ## Atomic writes
///
/// The CAS uses write-then-rename to ensure that writes are atomic. If a file
/// already exists, it is not written again. This is safe because the key is
/// computed from the content of the file, so if the file already exists it must
/// have the same content.
///
/// ## Key validation
///
/// While clients provide the key to the request, the CAS validates the key when
/// the content is written to ensure that the key provided by the user and the
/// key computed from the content actually match.
///
/// If they do not, this request is rejected and the write operation is aborted.
/// Making clients provide the key is due to two reasons:
/// 1. It reduces the chance that the client provides the wrong value.
/// 2. It allows this service to colocate the temporary file with the ultimate
///    destination for the content, which makes implementation simpler if we
///    move to multiple mounted disks for subsets of the CAS.
pub async fn handle(
    token: StatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(cas): Dep<Disk>,
    Dep(db): Dep<Postgres>,
    Path(key): Path<Key>,
    body: Body,
) -> CasWriteResponse {
    let stream = body.into_data_stream();
    let stream = stream.map_err(std::io::Error::other);
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
                error!(?err, user = ?token.user_id, org = ?token.org_id, "grant org access to cas key");
                return CasWriteResponse::Error(err);
            }

            keysets.organization(token.org_id).insert(key.clone());

            // We record access frequency asynchronously to avoid blocking
            // the overall request, since access frequency is a "nice to
            // have" feature while latency is a "must have" feature.
            tokio::spawn(async move {
                if let Err(err) = db.record_cas_key_access(token.user_id, &key).await {
                    warn!(?err, user = ?token.user_id, "record cas key access");
                }
            });

            CasWriteResponse::Created
        }
        Err(err) => {
            error!(?err, "write cas key");
            CasWriteResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CasWriteResponse {
    Created,
    Error(Report),
}

impl IntoResponse for CasWriteResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasWriteResponse::Created => StatusCode::CREATED.into_response(),
            CasWriteResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
