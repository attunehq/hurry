use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::{CargoRestoreRequest, CargoRestoreResponseTransport};
use color_eyre::eyre::Report;
use tap::Pipe;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[tracing::instrument(skip_all)]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoRestoreRequest>,
) -> CacheRestoreResponse {
    let org_id = match auth.require_org() {
        Ok(id) => id,
        Err((status, msg)) => return CacheRestoreResponse::Forbidden(status, msg),
    };

    match db.cargo_cache_restore(org_id, request).await {
        Ok(artifacts) if artifacts.is_empty() => {
            info!("cache.restore.miss");
            CacheRestoreResponse::NotFound
        }
        Ok(artifacts) => {
            info!("cache.restore.hit");
            artifacts
                .into_iter()
                .collect::<CargoRestoreResponseTransport>()
                .pipe(CacheRestoreResponse::Ok)
        }
        Err(err) => {
            error!(error = ?err, "cache.restore.error");
            CacheRestoreResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CacheRestoreResponse {
    Ok(CargoRestoreResponseTransport),
    NotFound,
    Forbidden(StatusCode, &'static str),
    Error(Report),
}

impl IntoResponse for CacheRestoreResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheRestoreResponse::Ok(body) => (StatusCode::OK, Json(body)).into_response(),
            CacheRestoreResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CacheRestoreResponse::Forbidden(status, msg) => (status, msg).into_response(),
            CacheRestoreResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
