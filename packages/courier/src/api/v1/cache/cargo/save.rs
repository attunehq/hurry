use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::CargoSaveRequest2;
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoSaveRequest2>,
) -> CacheSaveResponse {
    match db.cargo_cache_save(&auth, request).await {
        Ok(()) => {
            info!("cache.save.created");
            CacheSaveResponse::Created
        }
        Err(err) => {
            error!(error = ?err, "cache.save.error");
            CacheSaveResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CacheSaveResponse {
    Created,
    Error(Report),
}

impl IntoResponse for CacheSaveResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheSaveResponse::Created => StatusCode::CREATED.into_response(),
            CacheSaveResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
