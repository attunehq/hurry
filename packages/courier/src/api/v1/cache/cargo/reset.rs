use aerosol::axum::Dep;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use color_eyre::eyre::Report;
use tracing::{error, info, instrument};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[instrument(skip(auth))]
pub async fn handle(auth: AuthenticatedToken, Dep(db): Dep<Postgres>) -> CacheResetResponse {
    let org_id = match auth.require_org() {
        Ok(id) => id,
        Err((status, msg)) => return CacheResetResponse::Forbidden(status, msg),
    };

    // Delete the authenticated org's cache data
    match db.cargo_cache_reset(org_id).await {
        Ok(()) => {
            info!("cache.reset.success");
            CacheResetResponse::Success
        }
        Err(err) => {
            error!(error = ?err, "cache.reset.error");
            CacheResetResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CacheResetResponse {
    Success,
    Forbidden(StatusCode, &'static str),
    Error(Report),
}

impl IntoResponse for CacheResetResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheResetResponse::Success => StatusCode::NO_CONTENT.into_response(),
            CacheResetResponse::Forbidden(status, msg) => (status, msg).into_response(),
            CacheResetResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
