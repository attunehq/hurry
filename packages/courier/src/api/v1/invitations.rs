//! Invitation management endpoints.
//!
//! These endpoints allow organization admins to create and manage invitations,
//! and users to view and accept invitations.

use aerosol::axum::Dep;
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tap::Pipe;
use time::{Duration, OffsetDateTime};
use tracing::{error, info, warn};

use crate::{
    api::State,
    auth::{InvitationId, OrgId, OrgRole, SessionContext},
    crypto::generate_invitation_token,
    db::{AcceptInvitationResult, Postgres},
    rate_limit,
};

/// Invitations that live longer than this threshold are considered long-lived
/// and use more entropy in their tokens.
const LONG_LIVED_THRESHOLD: Duration = Duration::days(7);

pub fn organization_router() -> Router<State> {
    Router::new()
        .route("/{org_id}/invitations", post(create_invitation))
        .route("/{org_id}/invitations", get(list_invitations))
        .route(
            "/{org_id}/invitations/{invitation_id}",
            delete(revoke_invitation),
        )
}

pub fn router() -> Router<State> {
    let invitation = Router::new()
        .route("/{token}/accept", post(accept_invitation))
        .layer(rate_limit::invitation());

    Router::new()
        .route("/{token}", get(get_invitation_preview))
        .merge(invitation)
}

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
pub async fn create_invitation(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateInvitationRequest>,
) -> CreateInvitationResponse {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.create.not_admin"
            );
            return CreateInvitationResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.create.not_member"
            );
            return CreateInvitationResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "invitations.create.role_check_error");
            return CreateInvitationResponse::Error(err.to_string());
        }
    }

    let now = OffsetDateTime::now_utc();
    if let Some(exp) = request.expires_at
        && exp <= now
    {
        return CreateInvitationResponse::ExpiresAtInThePast;
    }

    if let Some(max) = request.max_uses
        && max < 1
    {
        return CreateInvitationResponse::MaxUsesLessThanOne;
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
            return CreateInvitationResponse::Error(err.to_string());
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

    CreateInvitationResponse::Created(CreateInvitationResponseBody {
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
pub enum CreateInvitationResponse {
    Created(CreateInvitationResponseBody),
    ExpiresAtInThePast,
    MaxUsesLessThanOne,
    Forbidden,
    Error(String),
}

impl IntoResponse for CreateInvitationResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateInvitationResponse::Created(body) => {
                (StatusCode::CREATED, Json(body)).into_response()
            }
            CreateInvitationResponse::ExpiresAtInThePast => {
                (StatusCode::BAD_REQUEST, "expires_at must be in the future").into_response()
            }
            CreateInvitationResponse::MaxUsesLessThanOne => {
                (StatusCode::BAD_REQUEST, "max_uses must be at least 1").into_response()
            }
            CreateInvitationResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can create invitations").into_response()
            }
            CreateInvitationResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InvitationListResponse {
    /// The list of invitations.
    pub invitations: Vec<InvitationEntry>,
}

#[derive(Debug, Serialize)]
pub struct InvitationEntry {
    /// The invitation ID.
    pub id: i64,

    /// The role to grant.
    pub role: OrgRole,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,

    /// The expiration timestamp. None means the invitation never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// The maximum number of uses.
    pub max_uses: Option<i32>,

    /// The number of times the invitation has been used.
    pub use_count: i32,

    /// Whether the invitation has been revoked.
    pub revoked: bool,
}

/// List invitations for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn list_invitations(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListInvitationsResponse {
    let org_id = OrgId::from_i64(org_id);

    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.list.not_admin"
            );
            return ListInvitationsResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.list.not_member"
            );
            return ListInvitationsResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "invitations.list.role_check_error");
            return ListInvitationsResponse::Error(err.to_string());
        }
    }

    match db.list_invitations(org_id).await {
        Ok(invitations) => {
            info!(
                org_id = %org_id,
                count = invitations.len(),
                "invitations.list.success"
            );
            invitations
                .into_iter()
                .map(|inv| InvitationEntry {
                    id: inv.id.as_i64(),
                    role: inv.role,
                    created_at: inv.created_at,
                    expires_at: inv.expires_at,
                    max_uses: inv.max_uses,
                    use_count: inv.use_count,
                    revoked: inv.revoked_at.is_some(),
                })
                .collect::<Vec<_>>()
                .pipe(|invitations| InvitationListResponse { invitations })
                .pipe(ListInvitationsResponse::Success)
        }
        Err(error) => {
            error!(?error, "invitations.list.error");
            ListInvitationsResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListInvitationsResponse {
    Success(InvitationListResponse),
    Forbidden,
    Error(String),
}

impl IntoResponse for ListInvitationsResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListInvitationsResponse::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            ListInvitationsResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can view invitations").into_response()
            }
            ListInvitationsResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

/// Revoke an invitation.
#[tracing::instrument(skip(db, session))]
pub async fn revoke_invitation(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, invitation_id)): Path<(i64, i64)>,
) -> RevokeInvitationResponse {
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
            return RevokeInvitationResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "invitations.revoke.not_member"
            );
            return RevokeInvitationResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "invitations.revoke.role_check_error");
            return RevokeInvitationResponse::Error(err.to_string());
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
            RevokeInvitationResponse::Success
        }
        Ok(false) => RevokeInvitationResponse::NotFound,
        Err(error) => {
            error!(?error, "invitations.revoke.error");
            RevokeInvitationResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum RevokeInvitationResponse {
    Success,
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for RevokeInvitationResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            RevokeInvitationResponse::Success => StatusCode::NO_CONTENT.into_response(),
            RevokeInvitationResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can revoke invitations").into_response()
            }
            RevokeInvitationResponse::NotFound => (
                StatusCode::NOT_FOUND,
                "Invitation not found or already revoked",
            )
                .into_response(),
            RevokeInvitationResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InvitationPreviewResponse {
    /// The organization name.
    pub organization_name: String,

    /// The role to grant.
    pub role: OrgRole,

    /// The expiration timestamp. None means the invitation never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,

    /// Whether the invitation is valid.
    pub valid: bool,
}

/// Get a preview of an invitation (no authentication required).
///
/// This allows potential members to see what organization they're joining
/// before signing in.
#[tracing::instrument(skip(db))]
pub async fn get_invitation_preview(
    Dep(db): Dep<Postgres>,
    Path(token): Path<String>,
) -> GetPreviewResponse {
    match db.get_invitation_preview(&token).await {
        Ok(Some(preview)) => {
            info!("invitations.preview.success");
            GetPreviewResponse::Success(InvitationPreviewResponse {
                organization_name: preview.organization_name,
                role: preview.role,
                expires_at: preview.expires_at,
                valid: preview.valid,
            })
        }
        Ok(None) => {
            warn!("invitations.preview.not_found");
            GetPreviewResponse::NotFound
        }
        Err(err) => {
            error!(?err, "invitations.preview.error");
            GetPreviewResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum GetPreviewResponse {
    Success(InvitationPreviewResponse),
    NotFound,
    Error(String),
}

impl IntoResponse for GetPreviewResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            GetPreviewResponse::Success(preview) => (StatusCode::OK, Json(preview)).into_response(),
            GetPreviewResponse::NotFound => {
                (StatusCode::NOT_FOUND, "Invitation not found").into_response()
            }
            GetPreviewResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

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
pub async fn accept_invitation(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(token): Path<String>,
) -> AcceptInvitationResponse {
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
            AcceptInvitationResponse::Success(AcceptInvitationResponseBody {
                organization_id: organization_id.as_i64(),
                organization_name,
                role,
            })
        }
        Ok(AcceptInvitationResult::NotFound) => {
            warn!(account_id = %session.account_id, "invitations.accept.not_found");
            AcceptInvitationResponse::NotFound
        }
        Ok(AcceptInvitationResult::Revoked) => {
            warn!(account_id = %session.account_id, "invitations.accept.revoked");
            AcceptInvitationResponse::Revoked
        }
        Ok(AcceptInvitationResult::Expired) => {
            warn!(account_id = %session.account_id, "invitations.accept.expired");
            AcceptInvitationResponse::Expired
        }
        Ok(AcceptInvitationResult::MaxUsesReached) => {
            warn!(account_id = %session.account_id, "invitations.accept.max_uses");
            AcceptInvitationResponse::MaxUsesReached
        }
        Ok(AcceptInvitationResult::AlreadyMember) => {
            warn!(account_id = %session.account_id, "invitations.accept.already_member");
            AcceptInvitationResponse::Conflict
        }
        Err(error) => {
            error!(?error, "invitations.accept.error");
            AcceptInvitationResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum AcceptInvitationResponse {
    Success(AcceptInvitationResponseBody),
    Revoked,
    Expired,
    MaxUsesReached,
    NotFound,
    Conflict,
    Error(String),
}

impl IntoResponse for AcceptInvitationResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            AcceptInvitationResponse::Success(body) => (StatusCode::OK, Json(body)).into_response(),
            AcceptInvitationResponse::Revoked => {
                (StatusCode::BAD_REQUEST, "This invitation has been revoked").into_response()
            }
            AcceptInvitationResponse::Expired => {
                (StatusCode::BAD_REQUEST, "This invitation has expired").into_response()
            }
            AcceptInvitationResponse::MaxUsesReached => (
                StatusCode::BAD_REQUEST,
                "This invitation has reached its maximum number of uses",
            )
                .into_response(),
            AcceptInvitationResponse::NotFound => {
                (StatusCode::NOT_FOUND, "Invitation not found").into_response()
            }
            AcceptInvitationResponse::Conflict => (
                StatusCode::CONFLICT,
                "You are already a member of this organization",
            )
                .into_response(),
            AcceptInvitationResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
