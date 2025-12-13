//! Organization management endpoints.

use aerosol::axum::Dep;
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tap::Pipe;
use time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::{
    api::{State, v1::invitations},
    auth::{AccountId, ApiKeyId, OrgId, OrgRole, SessionContext},
    db::Postgres,
    rate_limit,
};

pub fn router() -> Router<State> {
    let sensitive = Router::new()
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
        .merge(invitations::organization_router())
        .merge(sensitive)
}

#[derive(Debug, Deserialize)]
pub struct CreateOrganizationRequest {
    /// The organization name.
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrganizationResponse {
    /// The organization ID.
    pub id: i64,

    /// The organization name.
    pub name: String,
}

/// Create a new organization.
#[tracing::instrument(skip(db, session))]
pub async fn create_organization(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Json(request): Json<CreateOrganizationRequest>,
) -> CreateOrgResponse {
    if request.name.trim().is_empty() {
        return CreateOrgResponse::EmptyName;
    }

    let org_id = match db
        .create_organization_with_admin(&request.name, session.account_id)
        .await
    {
        Ok(id) => id,
        Err(error) => {
            error!(?error, "organizations.create.error");
            return CreateOrgResponse::Error(error.to_string());
        }
    };

    let _ = db
        .log_audit_event(
            Some(session.account_id),
            Some(org_id),
            "organization.created",
            Some(json!({ "name": request.name })),
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
    EmptyName,
    Error(String),
}

impl IntoResponse for CreateOrgResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateOrgResponse::Created(org) => (StatusCode::CREATED, Json(org)).into_response(),
            CreateOrgResponse::EmptyName => {
                (StatusCode::BAD_REQUEST, "Organization name cannot be empty").into_response()
            }
            CreateOrgResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MemberListResponse {
    /// The list of members.
    pub members: Vec<MemberEntry>,
}

#[derive(Debug, Serialize)]
pub struct MemberEntry {
    /// The account ID.
    pub account_id: i64,

    /// The account email.
    pub email: String,

    /// The account name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The member's role in the organization.
    pub role: OrgRole,

    /// The date the member joined the organization.
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
}

/// List members of an organization.
#[tracing::instrument(skip(db, session))]
pub async fn list_members(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListMembersResponse {
    let org_id = OrgId::from_i64(org_id);

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
        Err(error) => {
            error!(?error, "organizations.list_members.role_check_error");
            return ListMembersResponse::Error(error.to_string());
        }
    }

    match db.list_organization_members(org_id).await {
        Ok(members) => {
            info!(
                org_id = %org_id,
                count = members.len(),
                "organizations.list_members.success"
            );
            members
                .into_iter()
                .map(|m| MemberEntry {
                    account_id: m.account_id.as_i64(),
                    email: m.email,
                    name: m.name,
                    role: m.role,
                    joined_at: m.created_at,
                })
                .collect::<Vec<_>>()
                .pipe(|members| MemberListResponse { members })
                .pipe(ListMembersResponse::Success)
        }
        Err(error) => {
            error!(?error, "organizations.list_members.error");
            ListMembersResponse::Error(error.to_string())
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

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    /// The new role for the member.
    pub role: OrgRole,
}

/// Update a member's role in an organization.
#[tracing::instrument(skip(db, session))]
pub async fn update_member_role(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, target_account_id)): Path<(i64, i64)>,
    Json(request): Json<UpdateRoleRequest>,
) -> UpdateRoleResponse {
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
        Err(error) => {
            error!(?error, "organizations.update_role.role_check_error");
            return UpdateRoleResponse::Error(error.to_string());
        }
    }

    let current_role = match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) => role,
        Ok(None) => {
            return UpdateRoleResponse::NotFound;
        }
        Err(error) => {
            error!(?error, "organizations.update_role.target_check_error");
            return UpdateRoleResponse::Error(error.to_string());
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
                return UpdateRoleResponse::LastAdmin;
            }
            Ok(false) => {}
            Err(error) => {
                error!(?error, "organizations.update_role.last_admin_check_error");
                return UpdateRoleResponse::Error(error.to_string());
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
            UpdateRoleResponse::Success
        }
        Ok(false) => UpdateRoleResponse::NotFound,
        Err(error) => {
            error!(?error, "organizations.update_role.error");
            UpdateRoleResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum UpdateRoleResponse {
    Success,
    LastAdmin,
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for UpdateRoleResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            UpdateRoleResponse::Success => StatusCode::NO_CONTENT.into_response(),
            UpdateRoleResponse::LastAdmin => (
                StatusCode::BAD_REQUEST,
                "Cannot demote the last admin. Promote another member first.",
            )
                .into_response(),
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

/// Remove a member from an organization.
#[tracing::instrument(skip(db, session))]
pub async fn remove_member(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, target_account_id)): Path<(i64, i64)>,
) -> RemoveMemberResponse {
    let org_id = OrgId::from_i64(org_id);
    let target_account_id = AccountId::from_i64(target_account_id);

    if session.account_id == target_account_id {
        return RemoveMemberResponse::CannotRemoveSelf;
    }

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
        Err(error) => {
            error!(?error, "organizations.remove_member.role_check_error");
            return RemoveMemberResponse::Error(error.to_string());
        }
    }

    match db.get_member_role(org_id, target_account_id).await {
        Ok(Some(role)) if role.is_admin() => {
            match db.is_last_admin(org_id, target_account_id).await {
                Ok(true) => {
                    warn!(
                        org_id = %org_id,
                        target_account_id = %target_account_id,
                        "organizations.remove_member.last_admin"
                    );
                    return RemoveMemberResponse::LastAdmin;
                }
                Ok(false) => {}
                Err(error) => {
                    error!(?error, "organizations.remove_member.last_admin_check_error");
                    return RemoveMemberResponse::Error(error.to_string());
                }
            }
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            return RemoveMemberResponse::NotFound;
        }
        Err(error) => {
            error!(?error, "organizations.remove_member.target_check_error");
            return RemoveMemberResponse::Error(error.to_string());
        }
    }

    match db
        .remove_organization_member(org_id, target_account_id)
        .await
    {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "organization.member.removed",
                    Some(json!({
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
        Err(error) => {
            error!(?error, "organizations.remove_member.error");
            RemoveMemberResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum RemoveMemberResponse {
    Success,
    CannotRemoveSelf,
    LastAdmin,
    Forbidden,
    NotFound,
    Error(String),
}

impl IntoResponse for RemoveMemberResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            RemoveMemberResponse::Success => StatusCode::NO_CONTENT.into_response(),
            RemoveMemberResponse::CannotRemoveSelf => (
                StatusCode::BAD_REQUEST,
                "Cannot remove yourself. Use the leave endpoint instead.",
            )
                .into_response(),
            RemoveMemberResponse::LastAdmin => (
                StatusCode::BAD_REQUEST,
                "Cannot remove the last admin. Promote another member first.",
            )
                .into_response(),
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

/// Leave an organization.
#[tracing::instrument(skip(db, session))]
pub async fn leave_organization(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> LeaveOrgResponse {
    let org_id = OrgId::from_i64(org_id);

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
        Err(error) => {
            error!(?error, "organizations.leave.role_check_error");
            return LeaveOrgResponse::Error(error.to_string());
        }
    };

    if role.is_admin() {
        match db.is_last_admin(org_id, session.account_id).await {
            Ok(true) => {
                warn!(
                    account_id = %session.account_id,
                    org_id = %org_id,
                    "organizations.leave.last_admin"
                );
                return LeaveOrgResponse::LastAdmin;
            }
            Ok(false) => {}
            Err(error) => {
                error!(?error, "organizations.leave.last_admin_check_error");
                return LeaveOrgResponse::Error(error.to_string());
            }
        }
    }

    match db
        .remove_organization_member(org_id, session.account_id)
        .await
    {
        Ok(true) => {
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
        Err(error) => {
            error!(?error, "organizations.leave.error");
            LeaveOrgResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum LeaveOrgResponse {
    Success,
    LastAdmin,
    NotFound,
    Error(String),
}

impl IntoResponse for LeaveOrgResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            LeaveOrgResponse::Success => StatusCode::NO_CONTENT.into_response(),
            LeaveOrgResponse::LastAdmin => (
                StatusCode::BAD_REQUEST,
                "Cannot leave as the last admin. Promote another member first or delete the organization.",
            )
                .into_response(),
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

#[derive(Debug, Serialize)]
pub struct OrgApiKeyListResponse {
    /// The list of API keys.
    pub api_keys: Vec<OrgApiKeyEntry>,
}

#[derive(Debug, Serialize)]
pub struct OrgApiKeyEntry {
    /// The API key ID.
    pub id: i64,

    /// The API key name.
    pub name: String,

    /// The account ID of the key owner.
    pub account_id: i64,

    /// The email of the key owner.
    pub account_email: String,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,

    /// The last access timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub accessed_at: OffsetDateTime,
}

/// List API keys for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn list_org_api_keys(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListOrgApiKeysResponse {
    let org_id = OrgId::from_i64(org_id);

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
        Err(error) => {
            error!(?error, "organizations.api_keys.list.role_check_error");
            return ListOrgApiKeysResponse::Error(error.to_string());
        }
    }

    match db.list_all_org_api_keys(org_id).await {
        Ok(keys) => {
            info!(
                org_id = %org_id,
                count = keys.len(),
                "organizations.api_keys.list.success"
            );
            keys.into_iter()
                .map(|key| OrgApiKeyEntry {
                    id: key.id.as_i64(),
                    name: key.name,
                    account_id: key.account_id.as_i64(),
                    account_email: key.account_email,
                    created_at: key.created_at,
                    accessed_at: key.accessed_at,
                })
                .collect::<Vec<_>>()
                .pipe(|api_keys| OrgApiKeyListResponse { api_keys })
                .pipe(ListOrgApiKeysResponse::Success)
        }
        Err(error) => {
            error!(?error, "organizations.api_keys.list.error");
            ListOrgApiKeysResponse::Error(error.to_string())
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

#[derive(Debug, Deserialize)]
pub struct CreateOrgApiKeyRequest {
    /// The API key name.
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrgApiKeyResponse {
    /// The API key ID.
    pub id: i64,

    /// The API key name.
    pub name: String,

    /// The API key token. Only returned once at creation.
    pub token: String,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Create a new organization API key.
#[tracing::instrument(skip(db, session))]
pub async fn create_org_api_key(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateOrgApiKeyRequest>,
) -> CreateOrgApiKeyApiResponse {
    let org_id = OrgId::from_i64(org_id);

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
        Err(error) => {
            error!(?error, "organizations.api_keys.create.role_check_error");
            return CreateOrgApiKeyApiResponse::Error(error.to_string());
        }
    }

    let name = request.name.trim();
    if name.is_empty() {
        return CreateOrgApiKeyApiResponse::EmptyName;
    }

    match db.create_api_key(session.account_id, name, org_id).await {
        Ok((key_id, token)) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "api_key.created",
                    Some(json!({
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
                Err(error) => {
                    error!(?error, "organizations.api_keys.create.fetch_error");
                    CreateOrgApiKeyApiResponse::Error(error.to_string())
                }
            }
        }
        Err(error) => {
            error!(?error, "organizations.api_keys.create.error");
            CreateOrgApiKeyApiResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum CreateOrgApiKeyApiResponse {
    Created(CreateOrgApiKeyResponse),
    EmptyName,
    Forbidden,
    Error(String),
}

impl IntoResponse for CreateOrgApiKeyApiResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateOrgApiKeyApiResponse::Created(key) => {
                (StatusCode::CREATED, Json(key)).into_response()
            }
            CreateOrgApiKeyApiResponse::EmptyName => {
                (StatusCode::BAD_REQUEST, "API key name cannot be empty").into_response()
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
#[tracing::instrument(skip(db, session))]
pub async fn delete_org_api_key(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path((org_id, key_id)): Path<(i64, i64)>,
) -> DeleteOrgApiKeyResponse {
    let org_id = OrgId::from_i64(org_id);
    let key_id = ApiKeyId::from_i64(key_id);

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
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.role_check_error");
            return DeleteOrgApiKeyResponse::Error(error.to_string());
        }
    };

    let key = match db.get_api_key(key_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return DeleteOrgApiKeyResponse::NotFound,
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.fetch_error");
            return DeleteOrgApiKeyResponse::Error(error.to_string());
        }
    };

    if key.organization_id != org_id {
        return DeleteOrgApiKeyResponse::NotFound;
    }

    if key.revoked_at.is_some() {
        return DeleteOrgApiKeyResponse::NotFound;
    }

    if key.account_id != session.account_id && !user_role.is_admin() {
        return DeleteOrgApiKeyResponse::Forbidden;
    }

    match db.revoke_api_key(key_id).await {
        Ok(true) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "api_key.revoked",
                    Some(json!({
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
        Err(error) => {
            error!(?error, "organizations.api_keys.delete.error");
            DeleteOrgApiKeyResponse::Error(error.to_string())
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

#[derive(Debug, Deserialize)]
pub struct CreateBotRequest {
    /// The bot name.
    pub name: String,

    /// The email of the person/team responsible for this bot.
    pub responsible_email: String,
}

#[derive(Debug, Serialize)]
pub struct CreateBotResponse {
    /// The bot account ID.
    pub account_id: i64,

    /// The bot name.
    pub name: String,

    /// The API key token. Only returned once at creation.
    pub api_key: String,
}

/// Create a bot account for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn create_bot(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
    Json(request): Json<CreateBotRequest>,
) -> CreateBotApiResponse {
    let org_id = OrgId::from_i64(org_id);

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
        Err(error) => {
            error!(?error, "organizations.bots.create.role_check_error");
            return CreateBotApiResponse::Error(error.to_string());
        }
    }

    let name = request.name.trim();
    if name.is_empty() {
        return CreateBotApiResponse::EmptyName;
    }

    let email = request.responsible_email.trim();
    if email.is_empty() {
        return CreateBotApiResponse::EmptyEmail;
    }

    match db.create_bot_account(org_id, name, email).await {
        Ok((account_id, token)) => {
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    Some(org_id),
                    "bot.created",
                    Some(json!({
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
        Err(error) => {
            error!(?error, "organizations.bots.create.error");
            CreateBotApiResponse::Error(error.to_string())
        }
    }
}

#[derive(Debug)]
pub enum CreateBotApiResponse {
    Created(CreateBotResponse),
    EmptyName,
    EmptyEmail,
    Forbidden,
    Error(String),
}

impl IntoResponse for CreateBotApiResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateBotApiResponse::Created(bot) => (StatusCode::CREATED, Json(bot)).into_response(),
            CreateBotApiResponse::EmptyName => {
                (StatusCode::BAD_REQUEST, "Bot name cannot be empty").into_response()
            }
            CreateBotApiResponse::EmptyEmail => {
                (StatusCode::BAD_REQUEST, "Responsible email cannot be empty").into_response()
            }
            CreateBotApiResponse::Forbidden => {
                (StatusCode::FORBIDDEN, "Only admins can create bot accounts").into_response()
            }
            CreateBotApiResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BotListResponse {
    /// The list of bot accounts.
    pub bots: Vec<BotEntry>,
}

#[derive(Debug, Serialize)]
pub struct BotEntry {
    /// The bot account ID.
    pub account_id: i64,

    /// The bot name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The email of the person/team responsible for this bot.
    pub responsible_email: String,

    /// The creation timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List bot accounts for an organization.
#[tracing::instrument(skip(db, session))]
pub async fn list_bots(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(org_id): Path<i64>,
) -> ListBotsResponse {
    let org_id = OrgId::from_i64(org_id);

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
        Err(error) => {
            error!(?error, "organizations.bots.list.role_check_error");
            return ListBotsResponse::Error(error.to_string());
        }
    }

    match db.list_bot_accounts(org_id).await {
        Ok(bots) => {
            info!(
                org_id = %org_id,
                count = bots.len(),
                "organizations.bots.list.success"
            );
            bots.into_iter()
                .map(|bot| BotEntry {
                    account_id: bot.id.as_i64(),
                    name: bot.name,
                    responsible_email: bot.email,
                    created_at: bot.created_at,
                })
                .collect::<Vec<_>>()
                .pipe(|bots| BotListResponse { bots })
                .pipe(ListBotsResponse::Success)
        }
        Err(error) => {
            error!(?error, "organizations.bots.list.error");
            ListBotsResponse::Error(error.to_string())
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
