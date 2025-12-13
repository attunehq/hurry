//! Create organization API key endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, SessionContext},
    db::Postgres,
};

#[derive(Debug, Deserialize)]
pub struct CreateOrgApiKeyRequest {
    /// The API key name.
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrgApiKeyResponse {
    /// The API key ID.
    pub id: i64,

    /// The API key name.
    pub name: String,

    /// The API key token. Only returned once at creation.
    pub token: String,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Create a new organization API key.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateOrgApiKeyRequest>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.api_keys.create.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.api_keys.create.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    let name = request.name.trim();
    if name.is_empty() {
        return Response::EmptyName;
    }

    match db.create_api_key(session.account_id, name, org_id).await {
        Ok((key_id, token)) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "api_key.created",
                    Some(json!({
                        "key_id": key_id.as_i64(),
                        "name": name,
                        "type": "organization",
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                key_id = %key_id,
                "organizations.api_keys.create.success"
            );

            match db.get_api_key(key_id).await {
                Ok(Some(key)) => Response::Created(CreateOrgApiKeyResponse {
                    id: key.id.as_i64(),
                    name: key.name,
                    token: token.expose().to_string(),
                    created_at: key.created_at,
                }),
                Ok(None) => {
                    error!(key_id = %key_id, "organizations.api_keys.create.not_found_after_create");
                    Response::Error(String::from("Key not found after creation"))
                }
                Err(error) => {
                    error!(?error, "organizations.api_keys.create.fetch_error");
                    Response::Error(error.to_string())
                }
            }
        }
        Err(error) => {
            error!(?error, "organizations.api_keys.create.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Created(CreateOrgApiKeyResponse),
    EmptyName,
    Forbidden,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Created(key) => (StatusCode::CREATED, Json(key)).into_response(),
            Response::EmptyName => {
                (StatusCode::BAD_REQUEST, "API key name cannot be empty").into_response()
            }
            Response::Forbidden => (
                StatusCode::FORBIDDEN,
                "You must be a member of this organization to create API keys",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
