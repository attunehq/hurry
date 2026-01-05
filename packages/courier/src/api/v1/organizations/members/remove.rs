//! Remove organization member endpoint.

use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{AccountId, OrgId, SessionContext},
    db::Postgres,
};

/// Remove a member from an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, target_account_id)): Path<(i64, i64)>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);
    let target_account_id = AccountId::from_i64(target_account_id);

    if session.account_id == target_account_id {
        return Response::CannotRemoveSelf;
    }

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.remove_member.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.remove_member.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.remove_member.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) if role.is_admin() => {
            match db.is_last_admin(org_id, target_account_id).await {
                Ok(true) => {
                    warn!(
                        org_id = %org_id,
                        target_account_id = %target_account_id,
                        "organizations.remove_member.last_admin"
                    );
                    return Response::LastAdmin;
                }
                Ok(false) => {}
                Err(error) => {
                    error!(?error, "organizations.remove_member.last_admin_check_error");
                    return Response::Error(error.to_string());
                }
            }
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            return Response::NotFound;
        }
        Err(error) => {
            error!(?error, "organizations.remove_member.target_check_error");
            return Response::Error(error.to_string());
        }
    }

    // Revoke API keys BEFORE removing the member. Removing a member without
    // revoking their tokens is a security footgun: the member would no longer
    // appear in the org but could still access org resources with existing tokens.
    // If revocation fails, we abort the entire operation.
    let keys_revoked = match db
        .revoke_account_org_api_keys(target_account_id, org_id)
        .await
    {
        Ok(count) => count,
        Err(error) => {
            error!(?error, "organizations.remove_member.revoke_keys_error");
            return Response::Error(error.to_string());
        }
    };

    match db
        .remove_organization_member(org_id, target_account_id)
        .await
    {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.removed",
                    Some(json!({
                        "removed_account_id": target_account_id.as_i64(),
                        "api_keys_revoked": keys_revoked,
                    })),
                )
                .await;

            info!(
                org_id = %org_id,
                target_account_id = %target_account_id,
                keys_revoked = %keys_revoked,
                "organizations.remove_member.success"
            );
            Response::Success
        }
        Ok(false) => Response::NotFound,
        Err(error) => {
            error!(?error, "organizations.remove_member.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    CannotRemoveSelf,
    LastAdmin,
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::CannotRemoveSelf => (
                StatusCode::BAD_REQUEST,
                "Cannot remove yourself. Use the leave endpoint instead.",
            )
                .into_response(),
            Response::LastAdmin => (
                StatusCode::BAD_REQUEST,
                "Cannot remove the last admin. Promote another member first.",
            )
                .into_response(),
            Response::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can remove members").into_response()
            }
            Response::NotFound => (StatusCode::NOT_FOUND, "Member not found").into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
