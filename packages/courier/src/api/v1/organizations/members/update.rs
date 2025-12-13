//! Update member role endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{AccountId, OrgId, OrgRole, SessionContext},
    db::Postgres,
};

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    /// The new role for the member.
    pub role: OrgRole,
}

/// Update a member's role in an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, target_account_id)): Path<(i64, i64)>,
    Json(request): Json<UpdateRoleRequest>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);
    let target_account_id = AccountId::from_i64(target_account_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.update_role.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.update_role.not_member"
            );
            return Response::Forbidden;
        }
        Err(error) => {
            error!(?error, "organizations.update_role.role_check_error");
            return Response::Error(error.to_string());
        }
    }

    let current_role = match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            return Response::NotFound;
        }
        Err(error) => {
            error!(?error, "organizations.update_role.target_check_error");
            return Response::Error(error.to_string());
        }
    };

    if current_role.is_admin() && !request.role.is_admin() {
        match db.is_last_admin(org_id, target_account_id).await {
            Ok(true) => {
                warn!(
                    org_id = %org_id,
                    target_account_id = %target_account_id,
                    "organizations.update_role.last_admin"
                );
                return Response::LastAdmin;
            }
            Ok(false) => {}
            Err(error) => {
                error!(?error, "organizations.update_role.last_admin_check_error");
                return Response::Error(error.to_string());
            }
        }
    }

    match db
        .update_member_role(org_id, target_account_id, request.role)
        .await
    {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.role_updated",
                    Some(json!({
                        "target_account_id": target_account_id.as_i64(),
                        "new_role": request.role,
                    })),
                )
                .await;

            info!(
                org_id = %org_id,
                target_account_id = %target_account_id,
                new_role = %request.role,
                "organizations.update_role.success"
            );
            Response::Success
        }
        Ok(false) => Response::NotFound,
        Err(error) => {
            error!(?error, "organizations.update_role.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    LastAdmin,
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::LastAdmin => (
                StatusCode::BAD_REQUEST,
                "Cannot demote the last admin. Promote another member first.",
            )
                .into_response(),
            Response::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can update member roles").into_response()
            }
            Response::NotFound => (StatusCode::NOT_FOUND, "Member not found").into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
