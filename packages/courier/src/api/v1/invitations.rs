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
use time::{Duration, OffsetDateTime};
use tracing::{error, info, warn};

use crate::{
    api::State,
    auth::{InvitationId, OrgId, OrgRole, SessionContext},
    crypto::generate_invitation_token,
    db::{AcceptInvitationResult, Postgres},
    rate_limit,
};

/// Default invitation expiration: 7 days.
const DEFAULT_EXPIRATION_DAYS: i64 = 7;

/// Long-lived invitation threshold: 30 days.
const LONG_LIVED_THRESHOLD_DAYS: i64 = 30;

pub fn router() -> Router<State> {
    // Rate-limited routes (sensitive operations)
    let rate_limited = Router::new()
        .route("/invitations/{token}/accept", post(accept_invitation))
        .layer(rate_limit::sensitive());

    Router::new()
        // Organization-scoped endpoints (require admin)
        .route(
            "/organizations/{org_id}/invitations",
            post(create_invitation),
        )
        .route("/organizations/{org_id}/invitations", get(list_invitations))
        .route(
            "/organizations/{org_id}/invitations/{invitation_id}",
            delete(revoke_invitation),
        )
        // Public endpoints
        .route("/invitations/{token}", get(get_invitation_preview))
        // Merge rate-limited routes
        .merge(rate_limited)
}

// =============================================================================
// Create Invitation
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    /// Role to grant (defaults to "member").
    #[serde(default = "default_role")]
    pub role: OrgRole,
    /// Expiration in days from now (defaults to 7, max 365).
    #[serde(default = "default_expiration_days")]
    pub expires_in_days: i64,
    /// Maximum number of uses (None = unlimited).
    pub max_uses: Option<i32>,
}

fn default_role() -> OrgRole {
    OrgRole::Member
}

fn default_expiration_days() -> i64 {
    DEFAULT_EXPIRATION_DAYS
}

/// Create a new invitation for an organization.
///
/// Only admins can create invitations.
///
/// ## Endpoint
/// ```
/// POST /api/v1/organizations/{org_id}/invitations
/// Authorization: Bearer <session_token>
/// Content-Type: application/json
///
/// { "role": "member", "expires_in_days": 7, "max_uses": 10 }
/// ```
///
/// ## Responses
/// - 201: Invitation created
/// - 400: Invalid request
/// - 401: Not authenticated
/// - 403: Not an admin of the organization
#[tracing::instrument(skip(db, session))]
pub async fn create_invitation(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateInvitationRequest>,
) -> CreateInvitationResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is an admin
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

    // Validate expiration
    let expires_in_days = request.expires_in_days.clamp(1, 365);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(expires_in_days);

    // Validate max_uses
    if let Some(max) = request.max_uses
        && max < 1
    {
        return CreateInvitationResponse::BadRequest(String::from("max_uses must be at least 1"));
    }

    // Generate token (longer for long-lived invitations)
    let long_lived = expires_in_days > LONG_LIVED_THRESHOLD_DAYS;
    let token = generate_invitation_token(long_lived);

    // Create invitation
    let invitation_id = match db
        .create_invitation(
            org_id,
            &token,
            request.role,
            session.account_id,
            expires_at,
            request.max_uses,
        )
        .await
    {
        Ok(id) => id,
        Err(err) => {
            error!(?err, "invitations.create.error");
            return CreateInvitationResponse::Error(err.to_string());
        }
    };

    // Log audit event
    let _ = db
        .log_audit_event(
            Some(session.account_id),
            Some(org_id),
            "invitation.created",
            Some(serde_json::json!({
                "invitation_id": invitation_id.as_i64(),
                "role": request.role,
                "expires_in_days": expires_in_days,
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
        expires_at,
        max_uses: request.max_uses,
    })
}

#[derive(Debug, Serialize)]
pub struct CreateInvitationResponseBody {
    pub id: i64,
    pub token: String,
    pub role: OrgRole,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub max_uses: Option<i32>,
}

#[derive(Debug)]
pub enum CreateInvitationResponse {
    Created(CreateInvitationResponseBody),
    BadRequest(String),
    Forbidden,
    Error(String),
}

impl IntoResponse for CreateInvitationResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateInvitationResponse::Created(body) => {
                (StatusCode::CREATED, Json(body)).into_response()
            }
            CreateInvitationResponse::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, msg).into_response()
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

// =============================================================================
// List Invitations
// =============================================================================

#[derive(Debug, Serialize)]
pub struct InvitationListResponse {
    pub invitations: Vec<InvitationEntry>,
}

#[derive(Debug, Serialize)]
pub struct InvitationEntry {
    pub id: i64,
    pub role: OrgRole,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub max_uses: Option<i32>,
    pub use_count: i32,
    pub revoked: bool,
}

/// List invitations for an organization.
///
/// Only admins can view invitations.
///
/// ## Endpoint
/// ```
/// GET /api/v1/organizations/{org_id}/invitations
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of invitations
/// - 401: Not authenticated
/// - 403: Not an admin of the organization
#[tracing::instrument(skip(db, session))]
pub async fn list_invitations(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListInvitationsResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is an admin
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
            let entries = invitations
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
                .collect();
            ListInvitationsResponse::Success(InvitationListResponse {
                invitations: entries,
            })
        }
        Err(err) => {
            error!(?err, "invitations.list.error");
            ListInvitationsResponse::Error(err.to_string())
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

// =============================================================================
// Revoke Invitation
// =============================================================================

/// Revoke an invitation.
///
/// Only admins can revoke invitations.
///
/// ## Endpoint
/// ```
/// DELETE /api/v1/organizations/{org_id}/invitations/{invitation_id}
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 204: Invitation revoked
/// - 401: Not authenticated
/// - 403: Not an admin of the organization
/// - 404: Invitation not found
#[tracing::instrument(skip(db, session))]
pub async fn revoke_invitation(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, invitation_id)): Path<(i64, i64)>,
) -> RevokeInvitationResponse {
    let org_id = OrgId::from_i64(org_id);
    let invitation_id = InvitationId::from_i64(invitation_id);

    // Check if user is an admin
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
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "invitation.revoked",
                    Some(serde_json::json!({
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
        Err(err) => {
            error!(?err, "invitations.revoke.error");
            RevokeInvitationResponse::Error(err.to_string())
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

// =============================================================================
// Get Invitation Preview (Public)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct InvitationPreviewResponse {
    pub organization_name: String,
    pub role: OrgRole,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub valid: bool,
}

/// Get a preview of an invitation (no authentication required).
///
/// This allows potential members to see what organization they're joining
/// before signing in.
///
/// ## Endpoint
/// ```
/// GET /api/v1/invitations/{token}
/// ```
///
/// ## Responses
/// - 200: Invitation preview
/// - 404: Invitation not found
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

// =============================================================================
// Accept Invitation
// =============================================================================

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
///
/// ## Endpoint
/// ```
/// POST /api/v1/invitations/{token}/accept
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: Successfully joined organization
/// - 400: Invalid invitation (expired, revoked, max uses reached)
/// - 401: Not authenticated
/// - 404: Invitation not found
/// - 409: Already a member
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
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(organization_id),
                    "invitation.accepted",
                    Some(serde_json::json!({
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
            AcceptInvitationResponse::BadRequest(String::from("This invitation has been revoked"))
        }
        Ok(AcceptInvitationResult::Expired) => {
            warn!(account_id = %session.account_id, "invitations.accept.expired");
            AcceptInvitationResponse::BadRequest(String::from("This invitation has expired"))
        }
        Ok(AcceptInvitationResult::MaxUsesReached) => {
            warn!(account_id = %session.account_id, "invitations.accept.max_uses");
            AcceptInvitationResponse::BadRequest(String::from(
                "This invitation has reached its maximum number of uses",
            ))
        }
        Ok(AcceptInvitationResult::AlreadyMember) => {
            warn!(account_id = %session.account_id, "invitations.accept.already_member");
            AcceptInvitationResponse::Conflict
        }
        Err(err) => {
            error!(?err, "invitations.accept.error");
            AcceptInvitationResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum AcceptInvitationResponse {
    Success(AcceptInvitationResponseBody),
    BadRequest(String),
    NotFound,
    Conflict,
    Error(String),
}

impl IntoResponse for AcceptInvitationResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            AcceptInvitationResponse::Success(body) => (StatusCode::OK, Json(body)).into_response(),
            AcceptInvitationResponse::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, msg).into_response()
            }
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
