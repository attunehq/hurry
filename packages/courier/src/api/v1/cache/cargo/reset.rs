use aerosol::axum::Dep;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use color_eyre::eyre::Report;
use tracing::{error, info, instrument, warn};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[instrument(skip(auth))]
pub async fn handle(auth: AuthenticatedToken, Dep(db): Dep<Postgres>) -> CacheResetResponse {
    match db.get_member_role(auth.org_id, auth.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %auth.account_id,
                org_id = %auth.org_id,
                "cache.reset.not_admin"
            );
            return CacheResetResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %auth.account_id,
                org_id = %auth.org_id,
                "cache.reset.not_member"
            );
            return CacheResetResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "cache.reset.role_check_error");
            return CacheResetResponse::Error(err);
        }
    }

    match db.cargo_cache_reset(&auth).await {
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
    Forbidden,
    Error(Report),
}

impl IntoResponse for CacheResetResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheResetResponse::Success => StatusCode::NO_CONTENT.into_response(),
            CacheResetResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Admin access required").into_response()
            }
            CacheResetResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
