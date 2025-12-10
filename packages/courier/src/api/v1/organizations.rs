//! Organization management endpoints.
//!
//! These endpoints allow authenticated users to create organizations,
//! manage members, and leave organizations.

use aerosol::axum::Dep;
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    api::State,
    auth::{AccountId, OrgId, OrgRole, SessionContext},
    db::Postgres,
};

pub fn router() -> Router<State> {
    Router::new()
        .route("/", post(create_organization))
        .route("/{org_id}/members", get(list_members))
        .route("/{org_id}/members/{account_id}", patch(update_member_role))
        .route("/{org_id}/members/{account_id}", delete(remove_member))
        .route("/{org_id}/leave", post(leave_organization))
}

// =============================================================================
// Create Organization
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateOrganizationRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrganizationResponse {
    pub id: i64,
    pub name: String,
}

/// Create a new organization.
///
/// The authenticated user becomes the admin of the new organization.
///
/// ## Endpoint
/// ```
/// POST /api/v1/organizations
/// Authorization: Bearer <session_token>
/// Content-Type: application/json
///
/// { "name": "My Organization" }
/// ```
///
/// ## Responses
/// - 201: Organization created
/// - 400: Invalid request
/// - 401: Not authenticated
#[tracing::instrument(skip(db, session))]
pub async fn create_organization(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Json(request): Json<CreateOrganizationRequest>,
) -> CreateOrgResponse {
    if request.name.trim().is_empty() {
        return CreateOrgResponse::BadRequest(String::from("Organization name cannot be empty"));
    }

    // Create the organization
    let org_id = match db.create_organization(&request.name).await {
        Ok(id) => id,
        Err(err) => {
            error!(?err, "organizations.create.error");
            return CreateOrgResponse::Error(err.to_string());
        }
    };

    // Add the creator as admin
    if let Err(err) = db
        .add_organization_member(org_id, session.account_id, OrgRole::Admin)
        .await
    {
        error!(?err, "organizations.create.add_member_error");
        return CreateOrgResponse::Error(err.to_string());
    }

    // Log audit event
    let _ = db
        .log_audit_event(
            Some(session.account_id),
            Some(org_id),
            "organization.created",
            Some(serde_json::json!({ "name": request.name })),
        )
        .await;

    info!(
        account_id = %session.account_id,
        org_id = %org_id,
        "organizations.create.success"
    );

    CreateOrgResponse::Created(CreateOrganizationResponse {
        id: org_id.as_i64(),
        name: request.name,
    })
}

#[derive(Debug)]
pub enum CreateOrgResponse {
    Created(CreateOrganizationResponse),
    BadRequest(String),
    Error(String),
}

impl IntoResponse for CreateOrgResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateOrgResponse::Created(org) => (StatusCode::CREATED, Json(org)).into_response(),
            CreateOrgResponse::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            CreateOrgResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

// =============================================================================
// List Members
// =============================================================================

#[derive(Debug, Serialize)]
pub struct MemberListResponse {
    pub members: Vec<MemberEntry>,
}

#[derive(Debug, Serialize)]
pub struct MemberEntry {
    pub account_id: i64,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub role: OrgRole,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
}

/// List members of an organization.
///
/// Only members of the organization can view the member list.
///
/// ## Endpoint
/// ```
/// GET /api/v1/organizations/{org_id}/members
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of members
/// - 401: Not authenticated
/// - 403: Not a member of the organization
/// - 404: Organization not found
#[tracing::instrument(skip(db, session))]
pub async fn list_members(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListMembersResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is a member
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.list_members.not_member"
            );
            return ListMembersResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.list_members.role_check_error");
            return ListMembersResponse::Error(err.to_string());
        }
    }

    match db.list_organization_members(org_id).await {
        Ok(members) => {
            info!(
                org_id = %org_id,
                count = members.len(),
                "organizations.list_members.success"
            );
            let entries = members
                .into_iter()
                .map(|m| MemberEntry {
                    account_id: m.account_id.as_i64(),
                    email: m.email,
                    name: m.name,
                    role: m.role,
                    joined_at: m.created_at,
                })
                .collect();
            ListMembersResponse::Success(MemberListResponse { members: entries })
        }
        Err(err) => {
            error!(?err, "organizations.list_members.error");
            ListMembersResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListMembersResponse {
    Success(MemberListResponse),
    Forbidden,
    Error(String),
}

impl IntoResponse for ListMembersResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListMembersResponse::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            ListMembersResponse::Forbidden => (
                StatusCode::FORBIDDEN,
                "You must be a member of this organization to view members",
            )
                .into_response(),
            ListMembersResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

// =============================================================================
// Update Member Role
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role: OrgRole,
}

/// Update a member's role in an organization.
///
/// Only admins can update member roles.
///
/// ## Endpoint
/// ```
/// PATCH /api/v1/organizations/{org_id}/members/{account_id}
/// Authorization: Bearer <session_token>
/// Content-Type: application/json
///
/// { "role": "admin" | "member" }
/// ```
///
/// ## Responses
/// - 204: Role updated
/// - 400: Invalid request (e.g., demoting last admin)
/// - 401: Not authenticated
/// - 403: Not an admin of the organization
/// - 404: Member not found
#[tracing::instrument(skip(db, session))]
pub async fn update_member_role(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, target_account_id)): Path<(i64, i64)>,
    Json(request): Json<UpdateRoleRequest>,
) -> UpdateRoleResponse {
    let org_id = OrgId::from_i64(org_id);
    let target_account_id = AccountId::from_i64(target_account_id);

    // Check if user is an admin
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.update_role.not_admin"
            );
            return UpdateRoleResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.update_role.not_member"
            );
            return UpdateRoleResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.update_role.role_check_error");
            return UpdateRoleResponse::Error(err.to_string());
        }
    }

    // Check if target is a member
    let current_role = match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            return UpdateRoleResponse::NotFound;
        }
        Err(err) => {
            error!(?err, "organizations.update_role.target_check_error");
            return UpdateRoleResponse::Error(err.to_string());
        }
    };

    // If demoting from admin, check if they're the last admin
    if current_role.is_admin() && !request.role.is_admin() {
        match db.is_last_admin(org_id, target_account_id).await {
            Ok(true) => {
                warn!(
                    org_id = %org_id,
                    target_account_id = %target_account_id,
                    "organizations.update_role.last_admin"
                );
                return UpdateRoleResponse::BadRequest(String::from(
                    "Cannot demote the last admin. Promote another member first.",
                ));
            }
            Ok(false) => {}
            Err(err) => {
                error!(?err, "organizations.update_role.last_admin_check_error");
                return UpdateRoleResponse::Error(err.to_string());
            }
        }
    }

    // Update the role
    match db.update_member_role(org_id, target_account_id, request.role).await {
        Ok(true) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.role_updated",
                    Some(serde_json::json!({
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
            UpdateRoleResponse::Success
        }
        Ok(false) => UpdateRoleResponse::NotFound,
        Err(err) => {
            error!(?err, "organizations.update_role.error");
            UpdateRoleResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum UpdateRoleResponse {
    Success,
    BadRequest(String),
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for UpdateRoleResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            UpdateRoleResponse::Success => StatusCode::NO_CONTENT.into_response(),
            UpdateRoleResponse::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            UpdateRoleResponse::Forbidden => (
                StatusCode::FORBIDDEN,
                "Only admins can update member roles",
            )
                .into_response(),
            UpdateRoleResponse::NotFound => {
                (StatusCode::NOT_FOUND, "Member not found").into_response()
            }
            UpdateRoleResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

// =============================================================================
// Remove Member
// =============================================================================

/// Remove a member from an organization.
///
/// Only admins can remove members. Admins cannot remove themselves
/// (use leave endpoint instead) or the last admin.
///
/// ## Endpoint
/// ```
/// DELETE /api/v1/organizations/{org_id}/members/{account_id}
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 204: Member removed
/// - 400: Cannot remove last admin
/// - 401: Not authenticated
/// - 403: Not an admin or trying to remove yourself
/// - 404: Member not found
#[tracing::instrument(skip(db, session))]
pub async fn remove_member(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, target_account_id)): Path<(i64, i64)>,
) -> RemoveMemberResponse {
    let org_id = OrgId::from_i64(org_id);
    let target_account_id = AccountId::from_i64(target_account_id);

    // Cannot remove yourself via this endpoint
    if session.account_id == target_account_id {
        return RemoveMemberResponse::BadRequest(String::from(
            "Cannot remove yourself. Use the leave endpoint instead.",
        ));
    }

    // Check if user is an admin
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.remove_member.not_admin"
            );
            return RemoveMemberResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.remove_member.not_member"
            );
            return RemoveMemberResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.remove_member.role_check_error");
            return RemoveMemberResponse::Error(err.to_string());
        }
    }

    // Check if target is an admin and is the last admin
    match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) if role.is_admin() => {
            match db.is_last_admin(org_id, target_account_id).await {
                Ok(true) => {
                    warn!(
                        org_id = %org_id,
                        target_account_id = %target_account_id,
                        "organizations.remove_member.last_admin"
                    );
                    return RemoveMemberResponse::BadRequest(String::from(
                        "Cannot remove the last admin. Promote another member first.",
                    ));
                }
                Ok(false) => {}
                Err(err) => {
                    error!(?err, "organizations.remove_member.last_admin_check_error");
                    return RemoveMemberResponse::Error(err.to_string());
                }
            }
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            return RemoveMemberResponse::NotFound;
        }
        Err(err) => {
            error!(?err, "organizations.remove_member.target_check_error");
            return RemoveMemberResponse::Error(err.to_string());
        }
    }

    // Remove the member
    match db.remove_organization_member(org_id, target_account_id).await {
        Ok(true) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.removed",
                    Some(serde_json::json!({
                        "removed_account_id": target_account_id.as_i64(),
                    })),
                )
                .await;

            info!(
                org_id = %org_id,
                target_account_id = %target_account_id,
                "organizations.remove_member.success"
            );
            RemoveMemberResponse::Success
        }
        Ok(false) => RemoveMemberResponse::NotFound,
        Err(err) => {
            error!(?err, "organizations.remove_member.error");
            RemoveMemberResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum RemoveMemberResponse {
    Success,
    BadRequest(String),
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for RemoveMemberResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            RemoveMemberResponse::Success => StatusCode::NO_CONTENT.into_response(),
            RemoveMemberResponse::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            RemoveMemberResponse::Forbidden => (
                StatusCode::FORBIDDEN,
                "Only admins can remove members",
            )
                .into_response(),
            RemoveMemberResponse::NotFound => {
                (StatusCode::NOT_FOUND, "Member not found").into_response()
            }
            RemoveMemberResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

// =============================================================================
// Leave Organization
// =============================================================================

/// Leave an organization.
///
/// The last admin cannot leave. They must transfer ownership first.
///
/// ## Endpoint
/// ```
/// POST /api/v1/organizations/{org_id}/leave
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 204: Left organization
/// - 400: Cannot leave as last admin
/// - 401: Not authenticated
/// - 404: Not a member of the organization
#[tracing::instrument(skip(db, session))]
pub async fn leave_organization(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> LeaveOrgResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is a member and get their role
    let role = match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.leave.not_member"
            );
            return LeaveOrgResponse::NotFound;
        }
        Err(err) => {
            error!(?err, "organizations.leave.role_check_error");
            return LeaveOrgResponse::Error(err.to_string());
        }
    };

    // If admin, check if last admin
    if role.is_admin() {
        match db.is_last_admin(org_id, session.account_id).await {
            Ok(true) => {
                warn!(
                    account_id = %session.account_id,
                    org_id = %org_id,
                    "organizations.leave.last_admin"
                );
                return LeaveOrgResponse::BadRequest(String::from(
                    "Cannot leave as the last admin. Promote another member first or delete the organization.",
                ));
            }
            Ok(false) => {}
            Err(err) => {
                error!(?err, "organizations.leave.last_admin_check_error");
                return LeaveOrgResponse::Error(err.to_string());
            }
        }
    }

    // Remove the member
    match db
        .remove_organization_member(org_id, session.account_id)
        .await
    {
        Ok(true) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.left",
                    None,
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.leave.success"
            );
            LeaveOrgResponse::Success
        }
        Ok(false) => LeaveOrgResponse::NotFound,
        Err(err) => {
            error!(?err, "organizations.leave.error");
            LeaveOrgResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum LeaveOrgResponse {
    Success,
    BadRequest(String),
    NotFound,
    Error(String),
}

impl IntoResponse for LeaveOrgResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            LeaveOrgResponse::Success => StatusCode::NO_CONTENT.into_response(),
            LeaveOrgResponse::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            LeaveOrgResponse::NotFound => (
                StatusCode::NOT_FOUND,
                "You are not a member of this organization",
            )
                .into_response(),
            LeaveOrgResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
