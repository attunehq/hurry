//! Create invitation endpoint.

use aerosol::axum::Dep;
use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    auth::{OrgId, OrgRole, SessionContext},
    crypto::generate_invitation_token,
    db::Postgres,
};

use super::LONG_LIVED_THRESHOLD;

#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    /// Role to grant (defaults to "member").
    #[serde(default = "default_role")]
    pub role: OrgRole,

    /// Expiration timestamp. If omitted or null, the invitation never expires.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// Maximum number of uses (None = unlimited).
    pub max_uses: Option<i32>,
}

fn default_role() -> OrgRole {
    OrgRole::Member
}

/// Create a new invitation for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateInvitationRequest>,
) -> Response {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.create.not_admin"
            );
            return Response::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.create.not_member"
            );
            return Response::Forbidden;
        }
        Err(err) => {
            error!(?err, "invitations.create.role_check_error");
            return Response::Error(err.to_string());
        }
    }

    let now = OffsetDateTime::now_utc();
    if let Some(exp) = request.expires_at
        && exp <= now
    {
        return Response::ExpiresAtInThePast;
    }

    if let Some(max) = request.max_uses
        && max < 1
    {
        return Response::MaxUsesLessThanOne;
    }

    let long_lived = request
        .expires_at
        .map(|exp| (exp - now) > LONG_LIVED_THRESHOLD)
        .unwrap_or(true);
    let token = generate_invitation_token(long_lived);

    let invitation = db
        .create_invitation(
            org_id,
            &token,
            request.role,
            session.account_id,
            request.expires_at,
            request.max_uses,
        )
        .await;
    let invitation_id = match invitation {
        Ok(id) => id,
        Err(err) => {
            error!(?err, "invitations.create.error");
            return Response::Error(err.to_string());
        }
    };

    let _ = db
        .log_audit_event(
            Some(session.account_id),
            Some(org_id),
            "invitation.created",
            Some(json!({
                "invitation_id": invitation_id.as_i64(),
                "role": request.role,
                "expires_at": request.expires_at,
                "max_uses": request.max_uses,
            })),
        )
        .await;

    info!(
        org_id = %org_id,
        invitation_id = %invitation_id,
        "invitations.create.success"
    );

    Response::Created(CreateInvitationResponseBody {
        id: invitation_id.as_i64(),
        token,
        role: request.role,
        expires_at: request.expires_at,
        max_uses: request.max_uses,
    })
}

#[derive(Debug, Serialize)]
pub struct CreateInvitationResponseBody {
    /// The invitation ID.
    pub id: i64,

    /// The invitation token.
    pub token: String,

    /// The role to grant.
    pub role: OrgRole,

    /// The expiration timestamp.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// The maximum number of uses.
    pub max_uses: Option<i32>,
}

#[derive(Debug)]
pub enum Response {
    Created(CreateInvitationResponseBody),
    ExpiresAtInThePast,
    MaxUsesLessThanOne,
    Forbidden,
    Error(String),
}

impl IntoResponse for Response {
    fn into_response(self) -> axum::response::Response {
        match self {
            Response::Created(body) => (StatusCode::CREATED, Json(body)).into_response(),
            Response::ExpiresAtInThePast => {
                (StatusCode::BAD_REQUEST, "expires_at must be in the future").into_response()
            }
            Response::MaxUsesLessThanOne => {
                (StatusCode::BAD_REQUEST, "max_uses must be at least 1").into_response()
            }
            Response::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can create invitations").into_response()
            }
            Response::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}
