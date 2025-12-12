//! Current user endpoints.
//!
//! These endpoints allow authenticated users to view their own profile,
//! organization memberships, and manage their personal API keys.

use aerosol::axum::Dep;
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{error, info};

use crate::{
    api::State,
    auth::{ApiKeyId, OrgRole, SessionContext},
    db::Postgres,
    rate_limit,
};

pub fn router() -> Router<State> {
    // Rate-limited routes (sensitive operations)
    let rate_limited = Router::new()
        .route("/api-keys", post(create_api_key))
        .layer(rate_limit::sensitive());

    Router::new()
        .route("/", get(get_me))
        .route("/organizations", get(list_organizations))
        .route("/api-keys", get(list_api_keys))
        .route("/api-keys/{key_id}", delete(delete_api_key))
        .merge(rate_limited)
}

/// Response for GET /me endpoint.
#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub id: i64,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_username: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Get the current user's profile.
///
/// Returns the authenticated user's account information.
///
/// ## Endpoint
/// ```
/// GET /api/v1/me
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: User profile
/// - 401: Not authenticated
/// - 404: Account not found (shouldn't happen for valid sessions)
#[tracing::instrument(skip(db, session))]
pub async fn get_me(Dep(db): Dep<Postgres>, session: SessionContext) -> GetMeResponse {
    let account = match db.get_account(session.account_id).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            error!(account_id = %session.account_id, "me.get.not_found");
            return GetMeResponse::NotFound;
        }
        Err(err) => {
            error!(?err, "me.get.error");
            return GetMeResponse::Error(err.to_string());
        }
    };

    // Fetch GitHub username if linked
    let github_username = match db.get_github_identity(session.account_id).await {
        Ok(Some(identity)) => Some(identity.github_username),
        Ok(None) => None,
        Err(err) => {
            error!(?err, "me.get.github_identity_error");
            return GetMeResponse::Error(err.to_string());
        }
    };

    info!(account_id = %session.account_id, "me.get.success");
    GetMeResponse::Success(MeResponse {
        id: account.id.as_i64(),
        email: account.email,
        name: account.name,
        github_username,
        created_at: account.created_at,
    })
}

#[derive(Debug)]
pub enum GetMeResponse {
    Success(MeResponse),
    NotFound,
    Error(String),
}

impl IntoResponse for GetMeResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            GetMeResponse::Success(me) => (StatusCode::OK, Json(me)).into_response(),
            GetMeResponse::NotFound => (
                StatusCode::NOT_FOUND,
                "Account not found. This may indicate a database inconsistency.",
            )
                .into_response(),
            GetMeResponse::Error(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}

/// Response for GET /me/organizations endpoint.
#[derive(Debug, Serialize)]
pub struct OrganizationListResponse {
    pub organizations: Vec<OrganizationEntry>,
}

/// A single organization entry in the list response.
#[derive(Debug, Serialize)]
pub struct OrganizationEntry {
    pub id: i64,
    pub name: String,
    pub role: OrgRole,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// List the current user's organizations.
///
/// Returns all organizations the authenticated user is a member of,
/// along with their role in each organization.
///
/// ## Endpoint
/// ```
/// GET /api/v1/me/organizations
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of organizations with roles
/// - 401: Not authenticated
#[tracing::instrument(skip(db, session))]
pub async fn list_organizations(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
) -> ListOrganizationsResponse {
    match db.list_organizations_for_account(session.account_id).await {
        Ok(orgs) => {
            info!(
                account_id = %session.account_id,
                count = orgs.len(),
                "me.organizations.success"
            );
            let organizations = orgs
                .into_iter()
                .map(|org| OrganizationEntry {
                    id: org.organization.id.as_i64(),
                    name: org.organization.name,
                    role: org.role,
                    created_at: org.organization.created_at,
                })
                .collect();
            ListOrganizationsResponse::Success(OrganizationListResponse { organizations })
        }
        Err(err) => {
            error!(?err, "me.organizations.error");
            ListOrganizationsResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListOrganizationsResponse {
    Success(OrganizationListResponse),
    Error(String),
}

impl IntoResponse for ListOrganizationsResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListOrganizationsResponse::Success(list) => {
                (StatusCode::OK, Json(list)).into_response()
            }
            ListOrganizationsResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

// =============================================================================
// Personal API Key Management
// =============================================================================

/// Response for GET /me/api-keys endpoint.
#[derive(Debug, Serialize)]
pub struct ApiKeyListResponse {
    pub api_keys: Vec<ApiKeyEntry>,
}

/// A single API key entry (without the token value).
#[derive(Debug, Serialize)]
pub struct ApiKeyEntry {
    pub id: i64,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub accessed_at: OffsetDateTime,
}

/// List the current user's personal API keys.
///
/// Returns all personal (non-organization-scoped) API keys for the
/// authenticated user.
///
/// ## Endpoint
/// ```
/// GET /api/v1/me/api-keys
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 200: List of API keys
/// - 401: Not authenticated
#[tracing::instrument(skip(db, session))]
pub async fn list_api_keys(Dep(db): Dep<Postgres>, session: SessionContext) -> ListApiKeysResponse {
    match db.list_personal_api_keys(session.account_id).await {
        Ok(keys) => {
            info!(
                account_id = %session.account_id,
                count = keys.len(),
                "me.api_keys.list.success"
            );
            let api_keys = keys
                .into_iter()
                .map(|key| ApiKeyEntry {
                    id: key.id.as_i64(),
                    name: key.name,
                    created_at: key.created_at,
                    accessed_at: key.accessed_at,
                })
                .collect();
            ListApiKeysResponse::Success(ApiKeyListResponse { api_keys })
        }
        Err(err) => {
            error!(?err, "me.api_keys.list.error");
            ListApiKeysResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum ListApiKeysResponse {
    Success(ApiKeyListResponse),
    Error(String),
}

impl IntoResponse for ListApiKeysResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ListApiKeysResponse::Success(list) => (StatusCode::OK, Json(list)).into_response(),
            ListApiKeysResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

/// Request body for creating a personal API key.
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
}

/// Response for creating an API key (includes the token value).
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: i64,
    pub name: String,
    pub token: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Create a new personal API key.
///
/// Creates a personal (non-organization-scoped) API key for the authenticated
/// user. The token is only returned once in the response; it cannot be
/// retrieved later.
///
/// ## Endpoint
/// ```
/// POST /api/v1/me/api-keys
/// Authorization: Bearer <session_token>
/// Content-Type: application/json
///
/// {"name": "My API Key"}
/// ```
///
/// ## Responses
/// - 201: API key created (includes token)
/// - 400: Invalid request (empty name)
/// - 401: Not authenticated
#[tracing::instrument(skip(db, session))]
pub async fn create_api_key(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Json(request): Json<CreateApiKeyRequest>,
) -> CreateApiKeyApiResponse {
    let name = request.name.trim();
    if name.is_empty() {
        return CreateApiKeyApiResponse::BadRequest("API key name cannot be empty");
    }

    match db.create_api_key(session.account_id, name, None).await {
        Ok((key_id, token)) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    None,
                    "api_key.created",
                    Some(serde_json::json!({
                        "key_id": key_id.as_i64(),
                        "name": name,
                        "type": "personal",
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                key_id = %key_id,
                "me.api_keys.create.success"
            );
            // Fetch the key to get created_at
            match db.get_api_key(key_id).await {
                Ok(Some(key)) => CreateApiKeyApiResponse::Created(CreateApiKeyResponse {
                    id: key.id.as_i64(),
                    name: key.name,
                    token: token.expose().to_string(),
                    created_at: key.created_at,
                }),
                Ok(None) => {
                    error!(key_id = %key_id, "me.api_keys.create.not_found_after_create");
                    CreateApiKeyApiResponse::Error(String::from("Key not found after creation"))
                }
                Err(err) => {
                    error!(?err, "me.api_keys.create.fetch_error");
                    CreateApiKeyApiResponse::Error(err.to_string())
                }
            }
        }
        Err(err) => {
            error!(?err, "me.api_keys.create.error");
            CreateApiKeyApiResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum CreateApiKeyApiResponse {
    Created(CreateApiKeyResponse),
    BadRequest(&'static str),
    Error(String),
}

impl IntoResponse for CreateApiKeyApiResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CreateApiKeyApiResponse::Created(key) => {
                (StatusCode::CREATED, Json(key)).into_response()
            }
            CreateApiKeyApiResponse::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, msg).into_response()
            }
            CreateApiKeyApiResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

/// Delete a personal API key.
///
/// Revokes (soft-deletes) a personal API key. The key must belong to the
/// authenticated user and must be a personal key (not organization-scoped).
///
/// ## Endpoint
/// ```
/// DELETE /api/v1/me/api-keys/{key_id}
/// Authorization: Bearer <session_token>
/// ```
///
/// ## Responses
/// - 204: API key deleted
/// - 401: Not authenticated
/// - 403: Key belongs to another user or is org-scoped
/// - 404: API key not found
#[tracing::instrument(skip(db, session))]
pub async fn delete_api_key(
    Dep(db): Dep<Postgres>,
    session: SessionContext,
    Path(key_id): Path<i64>,
) -> DeleteApiKeyResponse {
    let key_id = ApiKeyId::from_i64(key_id);

    // Verify ownership and that it's a personal key
    match db.get_api_key(key_id).await {
        Ok(Some(key)) => {
            if key.account_id != session.account_id {
                return DeleteApiKeyResponse::Forbidden;
            }
            if key.organization_id.is_some() {
                return DeleteApiKeyResponse::Forbidden;
            }
            if key.revoked_at.is_some() {
                return DeleteApiKeyResponse::NotFound;
            }
        }
        Ok(None) => return DeleteApiKeyResponse::NotFound,
        Err(err) => {
            error!(?err, "me.api_keys.delete.fetch_error");
            return DeleteApiKeyResponse::Error(err.to_string());
        }
    }

    match db.revoke_api_key(key_id).await {
        Ok(true) => {
            // Log audit event
            let _ = db
                .log_audit_event(
                    Some(session.account_id),
                    None,
                    "api_key.revoked",
                    Some(serde_json::json!({
                        "key_id": key_id.as_i64(),
                        "type": "personal",
                    })),
                )
                .await;

            info!(
                account_id = %session.account_id,
                key_id = %key_id,
                "me.api_keys.delete.success"
            );
            DeleteApiKeyResponse::Deleted
        }
        Ok(false) => DeleteApiKeyResponse::NotFound,
        Err(err) => {
            error!(?err, "me.api_keys.delete.error");
            DeleteApiKeyResponse::Error(err.to_string())
        }
    }
}

#[derive(Debug)]
pub enum DeleteApiKeyResponse {
    Deleted,
    NotFound,
    Forbidden,
    Error(String),
}

impl IntoResponse for DeleteApiKeyResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            DeleteApiKeyResponse::Deleted => StatusCode::NO_CONTENT.into_response(),
            DeleteApiKeyResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            DeleteApiKeyResponse::Forbidden => StatusCode::FORBIDDEN.into_response(),
            DeleteApiKeyResponse::Error(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
