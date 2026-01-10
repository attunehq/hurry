//! Revoke invitation endpoint.

use aerosol::axum::Dep;
use axum::{extract::Path, http::StatusCode, response::IntoResponse};
use serde_json::json;
use tracing::{error, info};

use crate::{
    auth::{ApiError, InvitationId, OrgId, SessionContext},
    db::Postgres,
};

/// Revoke an invitation.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, invitation_id)): Path<(i64, i64)>,
) -> Result<Response, ApiError> {
    let org_id = OrgId::from_i64(org_id);
    let invitation_id = InvitationId::from_i64(invitation_id);

    // Verify admin access using strongly typed role check
    let admin = session.try_admin(&db, org_id).await?;

    match db.revoke_invitation(invitation_id).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(admin.account_id),
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
            Ok(Response::Success)
        }
        Ok(false) => Ok(Response::NotFound),
        Err(error) => {
            error!(?error, "invitations.revoke.error");
            Ok(Response::Error(error.to_string()))
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success,
    NotFound,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success => StatusCode::NO_CONTENT.into_response(),
            Response::NotFound => (
                StatusCode::NOT_FOUND,
                "Invitation not found or already revoked",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
