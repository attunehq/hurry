use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tracing::{info, warn};

use crate::{
    auth::{AuthenticatedStatelessToken, KeySets, OrgId, OrgKeySet, RawToken},
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
    Dep(keysets): Dep<KeySets>,
    Dep(db): Dep<Postgres>,
) -> MintStatelessResponse {
    match db.validate(org_id, token).await {
        Ok(None) => MintStatelessResponse::Unauthorized,
        Ok(Some(token)) => {
            let allowed = db
                .user_allowed_cas_keys(token.user_id, OrgKeySet::DEFAULT_LIMIT)
                .await;
            match allowed {
                Ok(allowed) => {
                    info!(allowed = ?allowed.len(), user = ?token.user_id, org = ?token.org_id, "inserting allowed cas keys");
                    keysets.organization(org_id).insert_all(allowed);
                }
                Err(error) => {
                    warn!(?error, user = ?token.user_id, "unable to get allowed cas keys for user");
                }
            }
            MintStatelessResponse::Success(token.into_stateless())
        }
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
