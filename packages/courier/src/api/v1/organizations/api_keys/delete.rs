//! Delete organization API key endpoint.

use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{ApiKeyId, OrgId, SessionContext},
    db::Postgres,
};

/// Delete an organization API key.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, key_id)): Path<(i64, i64)>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);
    let key_id = ApiKeyId::from_i64(key_id);

    let user_role = match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.api_keys.delete.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.role_check_error");
            return Response::Error(error.to_string());
        }
    };

    let key = match db.get_api_key(key_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return Response::NotFound,
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.fetch_error");
            return Response::Error(error.to_string());
        }
    };

    if key.organization_id != org_id {
        return Response::NotFound;
    }

    if key.revoked_at.is_some() {
        return Response::NotFound;
    }

    if key.account_id != session.account_id && !user_role.is_admin() {
        return Response::Forbidden;
    }

    match db.revoke_api_key(key_id).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
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
                account_id = %session.account_id,
                org_id = %org_id,
                key_id = %key_id,
                "organizations.api_keys.delete.success"
            );
            Response::Deleted
        }
        Ok(false) => Response::NotFound,
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Deleted,
    NotFound,
    Forbidden,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Deleted => StatusCode::NO_CONTENT.into_response(),
            Response::NotFound => StatusCode::NOT_FOUND.into_response(),
            Response::Forbidden => (
                StatusCode::FORBIDDEN,
                "Only admins or the key owner can delete API keys",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
