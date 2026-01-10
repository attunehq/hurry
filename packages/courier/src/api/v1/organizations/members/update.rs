//! Update member role endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{AccountId, ApiError, OrgId, OrgRole, SessionContext},
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
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);
    let target_account_id = AccountId::from_i64(target_account_id);

    // Verify admin access using strongly typed role check
    let admin = session.try_admin(&db, org_id).await?;

    let current_role = match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            return Ok(Response::NotFound);
        }
        Err(error) => {
            error!(?error, "organizations.update_role.target_check_error");
            return Ok(Response::Error(error.to_string()));
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
                return Ok(Response::LastAdmin);
            }
            Ok(false) => {}
            Err(error) => {
                error!(?error, "organizations.update_role.last_admin_check_error");
                return Ok(Response::Error(error.to_string()));
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
                    Some(admin.account_id),
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
            Ok(Response::Success)
        }
        Ok(false) => Ok(Response::NotFound),
        Err(error) => {
            error!(?error, "organizations.update_role.error");
            Ok(Response::Error(error.to_string()))
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
                "Cannot demote the last admin. Promote another member first.",
            )
                .into_response(),
            Response::NotFound => (StatusCode::NOT_FOUND, "Member not found").into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
