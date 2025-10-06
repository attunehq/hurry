use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tracing::error;

use crate::{
    api::v1::cas::check_allowed,
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::{Disk, Key},
};

/// Check whether the given key exists in the CAS.
///
/// ## TOCTOU (Time of Check Time of Use)
///
/// Normally, developers are advised to avoid "exists" checks since they are
/// prone to "TOCTOU" bugs: when you check if something exists, another process
/// or thread might alter the result (removing or adding the item) before you
/// then can act on the result of that check.
///
/// Here, we allow checking for existence because:
/// - If you check for existence before writing and it doesn't exist, and
///   another client does the same thing, writes are idempotent. The CAS always
///   writes items with a key deterministically derived from the value of the
///   content, so it's safe to write multiple times: at most all but one write
///   is wasted time and bandwidth. Not ideal, but okay.
/// - While we don't recommend checking this before reading (just try to read
///   the value instead), since content in the CAS is idempotent and stored
///   according to a key deterministically derived from the value of the content
///   it's always safe to check for existence before reading too even if another
///   client writes unconditionally.
/// - The exists check is mainly intended to allow clients to avoid having to
///   spend the time and bandwidth re-uploading content that already exists,
///   since this can be non-trivial. This tradeoff seems worth the minor amount
///   of extra complexity/potential confusion that having an existence check may
///   bring to the service.
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
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
) -> CasCheckResponse {
    match check_allowed(&keysets, &db, &key, &token).await {
        Ok(true) => {
            if cas.exists(&key).await {
                CasCheckResponse::Found
            } else {
                CasCheckResponse::NotFound
            }
        }
        Ok(false) => CasCheckResponse::NotFound,
        Err(err) => {
            error!(?err, "check allowed cas key");
            CasCheckResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CasCheckResponse {
    Found,
    NotFound,
    Error(Report),
}

impl IntoResponse for CasCheckResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasCheckResponse::Found => StatusCode::OK.into_response(),
            CasCheckResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CasCheckResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
