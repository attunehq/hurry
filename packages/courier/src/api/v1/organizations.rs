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
    auth::{AccountId, ApiKeyId, OrgId, OrgRole, SessionContext},
    db::Postgres,
    rate_limit,
};

pub fn router() -> Router<State> {
    // Rate-limited routes (sensitive operations)
    let rate_limited = Router::new()
        .route("/{org_id}/api-keys", post(create_org_api_key))
        .route("/{org_id}/bots", post(create_bot))
        .layer(rate_limit::sensitive());

    Router::new()
        .route("/", post(create_organization))
        .route("/{org_id}/members", get(list_members))
        .route("/{org_id}/members/{account_id}", patch(update_member_role))
        .route("/{org_id}/members/{account_id}", delete(remove_member))
        .route("/{org_id}/leave", post(leave_organization))
        .route("/{org_id}/api-keys", get(list_org_api_keys))
        .route("/{org_id}/api-keys/{key_id}", delete(delete_org_api_key))
        .route("/{org_id}/bots", get(list_bots))
        .merge(rate_limited)
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

    // Create the organization and add the creator as admin (atomically)
    let org_id = match db
        .create_organization_with_admin(&request.name, session.account_id)
        .await
    {
        Ok(id) => id,
        Err(err) => {
            error!(?err, "organizations.create.error");
            return CreateOrgResponse::Error(err.to_string());
        }
    };

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
    match db
        .update_member_role(org_id, target_account_id, request.role)
        .await
    {
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
            UpdateRoleResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can update member roles").into_response()
            }
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
    match db
        .remove_organization_member(org_id, target_account_id)
        .await
    {
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
            RemoveMemberResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can remove members").into_response()
            }
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

// =============================================================================
// Organization API Keys
// =============================================================================

/// Response for GET /organizations/{org_id}/api-keys endpoint.
#[derive(Debug, Serialize)]
pub struct OrgApiKeyListResponse {
    pub api_keys: Vec<OrgApiKeyEntry>,
}

/// A single organization API key entry.
#[derive(Debug, Serialize)]
pub struct OrgApiKeyEntry {
    pub id: i64,
    pub name: String,
    pub account_id: i64,
    pub account_email: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub accessed_at: OffsetDateTime,
}

/// List API keys for an organization.
///
/// Only members of the organization can view API keys.
/// All members can see all org-scoped keys (for transparency).
///
/// ## Endpoint
/// ```
/// GET /api/v1/organizations/{org_id}/api-keys
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of API keys
/// - 401: Not authenticated
/// - 403: Not a member of the organization
#[tracing::instrument(skip(db, session))]
pub async fn list_org_api_keys(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListOrgApiKeysResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is a member
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.api_keys.list.not_member"
            );
            return ListOrgApiKeysResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.api_keys.list.role_check_error");
            return ListOrgApiKeysResponse::Error(err.to_string());
        }
    }

    // List all org API keys (from all members)
    // We need to join with account to get emails
    match db.list_all_org_api_keys(org_id).await {
        Ok(keys) => {
            info!(
                org_id = %org_id,
                count = keys.len(),
                "organizations.api_keys.list.success"
            );
            let api_keys = keys
                .into_iter()
                .map(|key| OrgApiKeyEntry {
                    id: key.id.as_i64(),
                    name: key.name,
                    account_id: key.account_id.as_i64(),
                    account_email: key.account_email,
                    created_at: key.created_at,
                    accessed_at: key.accessed_at,
                })
                .collect();
            ListOrgApiKeysResponse::Success(OrgApiKeyListResponse { api_keys })
        }
        Err(err) => {
            error!(?err, "organizations.api_keys.list.error");
            ListOrgApiKeysResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListOrgApiKeysResponse {
    Success(OrgApiKeyListResponse),
    Forbidden,
    Error(String),
}

impl IntoResponse for ListOrgApiKeysResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListOrgApiKeysResponse::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            ListOrgApiKeysResponse::Forbidden => (
                StatusCode::FORBIDDEN,
                "You must be a member of this organization to view API keys",
            )
                .into_response(),
            ListOrgApiKeysResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

/// Request body for creating an organization API key.
#[derive(Debug, Deserialize)]
pub struct CreateOrgApiKeyRequest {
    pub name: String,
}

/// Response for creating an organization API key.
#[derive(Debug, Serialize)]
pub struct CreateOrgApiKeyResponse {
    pub id: i64,
    pub name: String,
    pub token: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Create a new organization API key.
///
/// Only members can create org-scoped API keys. The key is tied to both
/// the creator's account and the organization.
///
/// ## Endpoint
/// ```
/// POST /api/v1/organizations/{org_id}/api-keys
/// Authorization: Bearer <session_token>
/// Content-Type: application/json
///
/// {"name": "CI/CD Key"}
/// ```
///
/// ## Responses
/// - 201: API key created (includes token)
/// - 400: Invalid request (empty name)
/// - 401: Not authenticated
/// - 403: Not a member of the organization
#[tracing::instrument(skip(db, session))]
pub async fn create_org_api_key(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateOrgApiKeyRequest>,
) -> CreateOrgApiKeyApiResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is a member
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.api_keys.create.not_member"
            );
            return CreateOrgApiKeyApiResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.api_keys.create.role_check_error");
            return CreateOrgApiKeyApiResponse::Error(err.to_string());
        }
    }

    let name = request.name.trim();
    if name.is_empty() {
        return CreateOrgApiKeyApiResponse::BadRequest("API key name cannot be empty");
    }

    match db.create_api_key(session.account_id, name, org_id).await {
        Ok((key_id, token)) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "api_key.created",
                    Some(serde_json::json!({
                        "key_id": key_id.as_i64(),
                        "name": name,
                        "type": "organization",
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                key_id = %key_id,
                "organizations.api_keys.create.success"
            );
            // Fetch the key to get created_at
            match db.get_api_key(key_id).await {
                Ok(Some(key)) => CreateOrgApiKeyApiResponse::Created(CreateOrgApiKeyResponse {
                    id: key.id.as_i64(),
                    name: key.name,
                    token: token.expose().to_string(),
                    created_at: key.created_at,
                }),
                Ok(None) => {
                    error!(key_id = %key_id, "organizations.api_keys.create.not_found_after_create");
                    CreateOrgApiKeyApiResponse::Error(String::from("Key not found after creation"))
                }
                Err(err) => {
                    error!(?err, "organizations.api_keys.create.fetch_error");
                    CreateOrgApiKeyApiResponse::Error(err.to_string())
                }
            }
        }
        Err(err) => {
            error!(?err, "organizations.api_keys.create.error");
            CreateOrgApiKeyApiResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum CreateOrgApiKeyApiResponse {
    Created(CreateOrgApiKeyResponse),
    BadRequest(&'static str),
    Forbidden,
    Error(String),
}

impl IntoResponse for CreateOrgApiKeyApiResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateOrgApiKeyApiResponse::Created(key) => {
                (StatusCode::CREATED, Json(key)).into_response()
            }
            CreateOrgApiKeyApiResponse::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, msg).into_response()
            }
            CreateOrgApiKeyApiResponse::Forbidden => (
                StatusCode::FORBIDDEN,
                "You must be a member of this organization to create API keys",
            )
                .into_response(),
            CreateOrgApiKeyApiResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

/// Delete an organization API key.
///
/// Members can delete their own org API keys. Admins can delete any org API
/// key.
///
/// ## Endpoint
/// ```
/// DELETE /api/v1/organizations/{org_id}/api-keys/{key_id}
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 204: API key deleted
/// - 401: Not authenticated
/// - 403: Not authorized to delete this key
/// - 404: API key not found
#[tracing::instrument(skip(db, session))]
pub async fn delete_org_api_key(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, key_id)): Path<(i64, i64)>,
) -> DeleteOrgApiKeyResponse {
    let org_id = OrgId::from_i64(org_id);
    let key_id = ApiKeyId::from_i64(key_id);

    // Check user's role in the org
    let user_role = match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.api_keys.delete.not_member"
            );
            return DeleteOrgApiKeyResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.api_keys.delete.role_check_error");
            return DeleteOrgApiKeyResponse::Error(err.to_string());
        }
    };

    // Check key exists and belongs to this org
    let key = match db.get_api_key(key_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return DeleteOrgApiKeyResponse::NotFound,
        Err(err) => {
            error!(?err, "organizations.api_keys.delete.fetch_error");
            return DeleteOrgApiKeyResponse::Error(err.to_string());
        }
    };

    // Verify key belongs to this org
    if key.organization_id != org_id {
        return DeleteOrgApiKeyResponse::NotFound;
    }

    // Check already revoked
    if key.revoked_at.is_some() {
        return DeleteOrgApiKeyResponse::NotFound;
    }

    // Authorization: owner can delete their own key, admins can delete any key
    if key.account_id != session.account_id && !user_role.is_admin() {
        return DeleteOrgApiKeyResponse::Forbidden;
    }

    match db.revoke_api_key(key_id).await {
        Ok(true) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "api_key.revoked",
                    Some(serde_json::json!({
                        "key_id": key_id.as_i64(),
                        "key_owner_account_id": key.account_id.as_i64(),
                        "type": "organization",
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                key_id = %key_id,
                "organizations.api_keys.delete.success"
            );
            DeleteOrgApiKeyResponse::Deleted
        }
        Ok(false) => DeleteOrgApiKeyResponse::NotFound,
        Err(err) => {
            error!(?err, "organizations.api_keys.delete.error");
            DeleteOrgApiKeyResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum DeleteOrgApiKeyResponse {
    Deleted,
    NotFound,
    Forbidden,
    Error(String),
}

impl IntoResponse for DeleteOrgApiKeyResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            DeleteOrgApiKeyResponse::Deleted => StatusCode::NO_CONTENT.into_response(),
            DeleteOrgApiKeyResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            DeleteOrgApiKeyResponse::Forbidden => (
                StatusCode::FORBIDDEN,
                "Only admins or the key owner can delete API keys",
            )
                .into_response(),
            DeleteOrgApiKeyResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

// =============================================================================
// Bot Account Endpoints
// =============================================================================

/// Request body for creating a bot account.
#[derive(Debug, Deserialize)]
pub struct CreateBotRequest {
    /// Name of the bot (e.g., "CI Bot").
    pub name: String,
    /// Email of the person/team responsible for this bot.
    pub responsible_email: String,
}

/// Response for creating a bot account.
#[derive(Debug, Serialize)]
pub struct CreateBotResponse {
    pub account_id: i64,
    pub name: String,
    /// The API key token - only returned once at creation.
    pub api_key: String,
}

/// Create a bot account for an organization.
///
/// Bot accounts are organization-scoped accounts without GitHub identity,
/// for CI systems and automation. Only admins can create bot accounts.
///
/// ## Endpoint
/// ```
/// POST /api/v1/organizations/{org_id}/bots
/// Authorization: Bearer <session_token>
/// Content-Type: application/json
///
/// {
///   "name": "CI Bot",
///   "responsible_email": "alice@example.com"
/// }
/// ```
///
/// ## Responses
/// - 201: Bot account created (includes API key)
/// - 400: Invalid request
/// - 401: Not authenticated
/// - 403: Not an admin of the organization
#[tracing::instrument(skip(db, session))]
pub async fn create_bot(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateBotRequest>,
) -> CreateBotApiResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is an admin
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.create.not_admin"
            );
            return CreateBotApiResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.create.not_member"
            );
            return CreateBotApiResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.bots.create.role_check_error");
            return CreateBotApiResponse::Error(err.to_string());
        }
    }

    // Validate request
    let name = request.name.trim();
    if name.is_empty() {
        return CreateBotApiResponse::BadRequest("Bot name cannot be empty");
    }

    let email = request.responsible_email.trim();
    if email.is_empty() {
        return CreateBotApiResponse::BadRequest("Responsible email cannot be empty");
    }

    // Create the bot account
    match db.create_bot_account(org_id, name, email).await {
        Ok((account_id, token)) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "bot.created",
                    Some(serde_json::json!({
                        "bot_account_id": account_id.as_i64(),
                        "name": name,
                        "responsible_email": email,
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                org_id = %org_id,
                bot_account_id = %account_id,
                "organizations.bots.create.success"
            );

            CreateBotApiResponse::Created(CreateBotResponse {
                account_id: account_id.as_i64(),
                name: name.to_string(),
                api_key: token.expose().to_string(),
            })
        }
        Err(err) => {
            error!(?err, "organizations.bots.create.error");
            CreateBotApiResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum CreateBotApiResponse {
    Created(CreateBotResponse),
    BadRequest(&'static str),
    Forbidden,
    Error(String),
}

impl IntoResponse for CreateBotApiResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateBotApiResponse::Created(bot) => (StatusCode::CREATED, Json(bot)).into_response(),
            CreateBotApiResponse::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            CreateBotApiResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can create bot accounts").into_response()
            }
            CreateBotApiResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

/// Response for listing bot accounts.
#[derive(Debug, Serialize)]
pub struct BotListResponse {
    pub bots: Vec<BotEntry>,
}

/// A single bot account entry.
#[derive(Debug, Serialize)]
pub struct BotEntry {
    pub account_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub responsible_email: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List bot accounts for an organization.
///
/// Only admins can view bot accounts.
///
/// ## Endpoint
/// ```
/// GET /api/v1/organizations/{org_id}/bots
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of bot accounts
/// - 401: Not authenticated
/// - 403: Not an admin of the organization
#[tracing::instrument(skip(db, session))]
pub async fn list_bots(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListBotsResponse {
    let org_id = OrgId::from_i64(org_id);

    // Check if user is an admin
    match db.get_member_role(org_id, session.account_id).await {
        Ok(Some(role)) if role.is_admin() => {}
        Ok(Some(_)) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.list.not_admin"
            );
            return ListBotsResponse::Forbidden;
        }
        Ok(None) => {
            warn!(
                account_id = %session.account_id,
                org_id = %org_id,
                "organizations.bots.list.not_member"
            );
            return ListBotsResponse::Forbidden;
        }
        Err(err) => {
            error!(?err, "organizations.bots.list.role_check_error");
            return ListBotsResponse::Error(err.to_string());
        }
    }

    match db.list_bot_accounts(org_id).await {
        Ok(bots) => {
            info!(
                org_id = %org_id,
                count = bots.len(),
                "organizations.bots.list.success"
            );
            let entries = bots
                .into_iter()
                .map(|bot| BotEntry {
                    account_id: bot.id.as_i64(),
                    name: bot.name,
                    responsible_email: bot.email,
                    created_at: bot.created_at,
                })
                .collect();
            ListBotsResponse::Success(BotListResponse { bots: entries })
        }
        Err(err) => {
            error!(?err, "organizations.bots.list.error");
            ListBotsResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListBotsResponse {
    Success(BotListResponse),
    Forbidden,
    Error(String),
}

impl IntoResponse for ListBotsResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListBotsResponse::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            ListBotsResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can view bot accounts").into_response()
            }
            ListBotsResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
