use aerosol::axum::Dep;
use axum::http::StatusCode;
use tracing::{error, info, instrument};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[instrument(skip(auth))]
pub async fn handle(auth: AuthenticatedToken, Dep(db): Dep<Postgres>) -> StatusCode {
    // Delete the authenticated org's cache data
    match db.cargo_cache_reset(&auth).await {
        Ok(()) => {
            info!("cache.reset.success");
            StatusCode::NO_CONTENT
        }
        Err(err) => {
            error!(error = ?err, "cache.reset.error");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
