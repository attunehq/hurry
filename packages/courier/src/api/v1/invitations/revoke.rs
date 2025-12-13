//! Revoke invitation endpoint.

use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{InvitationId, OrgId, SessionContext},
    db::Postgres,
};

/// Revoke an invitation.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, invitation_id)): Path<(i64, i64)>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);
    let invitation_id = InvitationId::from_i64(invitation_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.revoke.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.revoke.not_member"
            );
            return Response::Forbidden;
        }
        Err(err) => {
            error!(?err, "invitations.revoke.role_check_error");
            return Response::Error(err.to_string());
        }
    }

    match db.revoke_invitation(invitation_id).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "invitation.revoked",
                    Some(json!({
                        "invitation_id": invitation_id.as_i64(),
                    })),
                )
                .await;

            info!(
                org_id = %org_id,
                invitation_id = %invitation_id,
                "invitations.revoke.success"
            );
            Response::Success
        }
        Ok(false) => Response::NotFound,
        Err(error) => {
            error!(?error, "invitations.revoke.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can revoke invitations").into_response()
            }
            Response::NotFound => (
                StatusCode::NOT_FOUND,
                "Invitation not found or already revoked",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
