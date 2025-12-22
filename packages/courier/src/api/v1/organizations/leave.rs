//! Leave organization endpoint.

use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, SessionContext},
    db::Postgres,
};

/// Leave an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    let role = match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.leave.not_member"
            );
            return Response::NotFound;
        }
        Err(error) => {
            error!(?error, "organizations.leave.role_check_error");
            return Response::Error(error.to_string());
        }
    };

    if role.is_admin() {
        match db.is_last_admin(org_id, session.account_id).await {
            Ok(true) => {
                warn!(
                    account_id = %session.account_id,
                    org_id = %org_id,
                    "organizations.leave.last_admin"
                );
                return Response::LastAdmin;
            }
            Ok(false) => {}
            Err(error) => {
                error!(?error, "organizations.leave.last_admin_check_error");
                return Response::Error(error.to_string());
            }
        }
    }

    // Revoke API keys BEFORE removing the member. Removing a member without
    // revoking their tokens is a security footgun: the member would no longer
    // appear in the org but could still access org resources with existing tokens.
    // If revocation fails, we abort the entire operation.
    let keys_revoked = match db
        .revoke_account_org_api_keys(session.account_id, org_id)
        .await
    {
        Ok(count) => count,
        Err(error) => {
            error!(?error, "organizations.leave.revoke_keys_error");
            return Response::Error(error.to_string());
        }
    };

    match db
        .remove_organization_member(org_id, session.account_id)
        .await
    {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.left",
                    Some(serde_json::json!({
                        "api_keys_revoked": keys_revoked,
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                keys_revoked = %keys_revoked,
                "organizations.leave.success"
            );
            Response::Success
        }
        Ok(false) => Response::NotFound,
        Err(error) => {
            error!(?error, "organizations.leave.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    LastAdmin,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::LastAdmin => (
                StatusCode::BAD_REQUEST,
                "Cannot leave as the last admin. Promote another member first or delete the organization.",
            )
                .into_response(),
            Response::NotFound => (
                StatusCode::NOT_FOUND,
                "You are not a member of this organization",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
