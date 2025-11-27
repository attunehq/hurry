use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{
    auth::AuthenticatedToken,
    db::Postgres,
    storage::{Disk, Key},
};

/// Check whether the given key exists in the CAS.
///
/// This handler implements the HEAD endpoint for checking blob existence
/// without downloading the full content.
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
#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
) -> CasCheckResponse {
    // Check if org has access to this CAS key
    // Return NotFound (not Forbidden) to avoid leaking information about blob
    // existence
    match db.check_cas_access(auth.org_id, &key).await {
        Ok(true) => {}
        Ok(false) => {
            info!("cas.check.no_access");
            return CasCheckResponse::NotFound;
        }
        Err(err) => {
            error!(error = ?err, "cas.check.access_check_error");
            return CasCheckResponse::Error(err);
        }
    }

    // Check if blob exists
    match cas.exists(&key).await {
        Ok(true) => {
            info!("cas.check.found");
            CasCheckResponse::Found
        }
        Ok(false) => {
            info!("cas.check.not_found");
            CasCheckResponse::NotFound
        }
        Err(err) => {
            error!(error = ?err, "cas.check.error");
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
