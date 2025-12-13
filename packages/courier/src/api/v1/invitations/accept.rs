//! Accept invitation endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgRole, SessionContext},
    db::{AcceptInvitationResult, Postgres},
};

#[derive(Debug, Serialize)]
pub struct AcceptInvitationResponseBody {
    pub organization_id: i64,
    pub organization_name: String,
    pub role: OrgRole,
}

/// Accept an invitation and join an organization.
///
/// Requires authentication. The authenticated user will be added to the
/// organization with the role specified in the invitation.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(token): Path<String>,
) -> Response {
    match db.accept_invitation(&token, session.account_id).await {
        Ok(AcceptInvitationResult::Success {
            organization_id,
            organization_name,
            role,
        }) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(organization_id),
                    "invitation.accepted",
                    Some(json!({
                        "role": role,
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %organization_id,
                "invitations.accept.success"
            );
            Response::Success(AcceptInvitationResponseBody {
                organization_id: organization_id.as_i64(),
                organization_name,
                role,
            })
        }
        Ok(AcceptInvitationResult::NotFound) => {
            warn!(account_id = %session.account_id, "invitations.accept.not_found");
            Response::NotFound
        }
        Ok(AcceptInvitationResult::Revoked) => {
            warn!(account_id = %session.account_id, "invitations.accept.revoked");
            Response::Revoked
        }
        Ok(AcceptInvitationResult::Expired) => {
            warn!(account_id = %session.account_id, "invitations.accept.expired");
            Response::Expired
        }
        Ok(AcceptInvitationResult::MaxUsesReached) => {
            warn!(account_id = %session.account_id, "invitations.accept.max_uses");
            Response::MaxUsesReached
        }
        Ok(AcceptInvitationResult::AlreadyMember) => {
            warn!(account_id = %session.account_id, "invitations.accept.already_member");
            Response::Conflict
        }
        Err(error) => {
            error!(?error, "invitations.accept.error");
            Response::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Success(AcceptInvitationResponseBody),
    Revoked,
    Expired,
    MaxUsesReached,
    NotFound,
    Conflict,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Success(body) => (StatusCode::OK, Json(body)).into_response(),
            Response::Revoked => {
                (StatusCode::BAD_REQUEST, "This invitation has been revoked").into_response()
            }
            Response::Expired => {
                (StatusCode::BAD_REQUEST, "This invitation has expired").into_response()
            }
            Response::MaxUsesReached => (
                StatusCode::BAD_REQUEST,
                "This invitation has reached its maximum number of uses",
            )
                .into_response(),
            Response::NotFound => (StatusCode::NOT_FOUND, "Invitation not found").into_response(),
            Response::Conflict => (
                StatusCode::CONFLICT,
                "You are already a member of this organization",
            )
                .into_response(),
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
