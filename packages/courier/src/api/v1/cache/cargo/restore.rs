use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::{CargoRestoreRequest, CargoRestoreResponse};
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[tracing::instrument(skip_all)]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoRestoreRequest>,
) -> CacheRestoreResponse {
    match db.cargo_cache_restore(&auth, request).await {
        Ok(artifacts) if artifacts.is_empty() => {
            info!("cache.restore.miss");
            CacheRestoreResponse::NotFound
        }
        Ok(artifacts) => {
            info!("cache.restore.hit");
            CacheRestoreResponse::Ok(CargoRestoreResponse::new(artifacts))
        }
        Err(err) => {
            error!(error = ?err, "cache.restore.error");
            CacheRestoreResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CacheRestoreResponse {
    Ok(CargoRestoreResponse),
    NotFound,
    Error(Report),
}

impl IntoResponse for CacheRestoreResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheRestoreResponse::Ok(body) => (StatusCode::OK, Json(body)).into_response(),
            CacheRestoreResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CacheRestoreResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
