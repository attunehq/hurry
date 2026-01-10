//! List organization API keys endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{
    auth::{ApiError, OrgId, SessionContext},
    db::Postgres,
};

#[derive(Debug, Serialize)]
pub struct OrgApiKeyListResponse {
    /// The list of API keys.
    pub api_keys: Vec<OrgApiKeyEntry>,
}

#[derive(Debug, Serialize)]
pub struct OrgApiKeyEntry {
    /// The API key ID.
    pub id: i64,

    /// The API key name.
    pub name: String,

    /// The account ID of the key owner.
    pub account_id: i64,

    /// The email of the key owner.
    pub account_email: String,

    /// Whether the key owner is a bot (i.e., does not have a GitHub identity).
    pub bot: bool,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,

    /// The last access timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub accessed_at: OffsetDateTime,
}

/// List API keys for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);

    // Verify membership using strongly typed role check
    let _member = session.try_member(&db, org_id).await?;

    match db.list_all_org_api_keys(org_id).await {
        Ok(keys) => {
            info!(
                org_id = %org_id,
                count = keys.len(),
                "organizations.api_keys.list.success"
            );
            Ok(keys
                .into_iter()
                .map(|key| OrgApiKeyEntry {
                    id: key.id.as_i64(),
                    name: key.name,
                    account_id: key.account_id.as_i64(),
                    account_email: key.account_email,
                    bot: !key.has_github_identity,
                    created_at: key.created_at,
                    accessed_at: key.accessed_at,
                })
                .collect::<Vec<_>>()
                .pipe(|api_keys| OrgApiKeyListResponse { api_keys })
                .pipe(Response::Success))
        }
        Err(error) => {
            error!(?error, "organizations.api_keys.list.error");
            Ok(Response::Error(error.to_string()))
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(OrgApiKeyListResponse),
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
