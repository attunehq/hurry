use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;

use crate::{
    auth::{AuthenticatedStatelessToken, OrgId, RawToken},
    db::Postgres,
};

#[derive(Debug)]
pub enum MintStatelessResponse {
    Unauthorized,
    Success(AuthenticatedStatelessToken),
    Error(Report),
}

pub async fn handle(
    token: RawToken,
    org_id: OrgId,
    Dep(db): Dep<Postgres>,
) -> MintStatelessResponse {
    let org_id = org_id.into();
    match db.validate(org_id, token).await {
        Ok(None) => MintStatelessResponse::Unauthorized,
        Ok(Some(token)) => MintStatelessResponse::Success(token.into_stateless()),
        Err(error) => MintStatelessResponse::Error(error),
    }
}

impl IntoResponse for MintStatelessResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            MintStatelessResponse::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            MintStatelessResponse::Success(stateless) => {
                (StatusCode::OK, Json(stateless)).into_response()
            }
            MintStatelessResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}
