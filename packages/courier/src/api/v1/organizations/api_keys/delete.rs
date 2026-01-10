//! Delete organization API key endpoint.

use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use serde_json::json;
use tracing::{error, info};

use crate::{
    auth::{ApiError, ApiKeyId, OrgId, SessionContext},
    db::Postgres,
};

/// Delete an organization API key.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, key_id)): Path<(i64, i64)>,
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);
    let key_id = ApiKeyId::from_i64(key_id);

    // Verify membership using strongly typed role check
    let member = session.try_member(&db, org_id).await?;

    let key = match db.get_api_key(key_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return Ok(Response::NotFound),
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.fetch_error");
            return Ok(Response::Error(error.to_string()));
        }
    };

    if key.organization_id != org_id {
        return Ok(Response::NotFound);
    }

    if key.revoked_at.is_some() {
        return Ok(Response::NotFound);
    }

    // Check if user can delete: must be key owner or admin
    if key.account_id != member.account_id && !member.role.is_admin() {
        return Err(ApiError::Forbidden(
            "Only admins or the key owner can delete API keys",
        ));
    }

    match db.revoke_api_key(key_id).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(member.account_id),
                    Some(org_id),
                    "api_key.revoked",
                    Some(json!({
                        "key_id": key_id.as_i64(),
                        "key_owner_account_id": key.account_id.as_i64(),
                        "type": "organization",
                    })),
                )
                .await;

            info!(
                account_id = %member.account_id,
                org_id = %org_id,
                key_id = %key_id,
                "organizations.api_keys.delete.success"
            );
            Ok(Response::Deleted)
        }
        Ok(false) => Ok(Response::NotFound),
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.error");
            Ok(Response::Error(error.to_string()))
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Deleted,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Deleted => StatusCode::NO_CONTENT.into_response(),
            Response::NotFound => StatusCode::NOT_FOUND.into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
