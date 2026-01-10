use aerosol::axum::Dep;
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use derive_more::{Debug, Display};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{api, db};

/// Organization role for membership.
///
/// This enum maps to the `organization_role` table in the database.
/// New roles should be added both here and in the database.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    /// Regular organization member with basic access.
    Member,

    /// Organization administrator with full permissions.
    Admin,
}

impl OrgRole {
    /// Database role name.
    pub fn as_db_name(&self) -> &'static str {
        match self {
            OrgRole::Member => "member",
            OrgRole::Admin => "admin",
        }
    }

    /// Parse a role from its database name.
    pub fn from_db_name(name: &str) -> Option<Self> {
        match name {
            "member" => Some(OrgRole::Member),
            "admin" => Some(OrgRole::Admin),
            _ => None,
        }
    }

    /// Check for admin privileges.
    pub fn is_admin(&self) -> bool {
        matches!(self, OrgRole::Admin)
    }
}

/// An ID uniquely identifying an organization.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
pub struct OrgId(i64);

impl OrgId {
    pub fn as_i64(&self) -> i64 {
        self.0
    }

    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }
}

/// An ID uniquely identifying an account.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
pub struct AccountId(i64);

impl AccountId {
    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }

    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

/// An ID uniquely identifying an invitation.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
pub struct InvitationId(i64);

impl InvitationId {
    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }

    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

/// An ID uniquely identifying a user session.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
pub struct SessionId(i64);

impl SessionId {
    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }

    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

/// An ID uniquely identifying an API key.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
pub struct ApiKeyId(i64);

impl ApiKeyId {
    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }

    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

/// A raw token which has not yet been validated against the database.
///
/// The main intent for this type is to prevent leaking the token in logs
/// accidentally; users should generally interact with [`AuthenticatedToken`]
/// instead.
///
/// Importantly, this type _will_ successfully serialize and deserialize; the
/// intention for this is to support the server sending the raw token back to
/// the client when one is generated.
///
/// To view the token's value, use the `expose` method.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
#[debug("[redacted]")]
#[display("[redacted]")]
pub struct RawToken(String);

impl RawToken {
    /// Create a new instance from arbitrary text.
    pub fn new(value: impl Into<String>) -> Self {
        RawToken(value.into())
    }

    /// View the interior value of the token.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Generate a new raw token.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        RawToken::new(hex::encode(bytes))
    }
}

impl From<AuthenticatedToken> for RawToken {
    fn from(token: AuthenticatedToken) -> Self {
        token.plaintext
    }
}

impl AsRef<RawToken> for RawToken {
    fn as_ref(&self) -> &RawToken {
        self
    }
}

impl From<&RawToken> for RawToken {
    fn from(token: &RawToken) -> Self {
        token.clone()
    }
}

/// A session token for web UI authentication.
///
/// Similar to [`RawToken`] but specifically for user sessions. Session tokens
/// have higher entropy (256 bits vs 128 bits for API keys) and are used for
/// web UI authentication via OAuth.
///
/// Like `RawToken`, this type prevents leaking the token in logs accidentally.
/// The token serializes/deserializes to support returning it to the client.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
#[debug("[redacted]")]
#[display("[redacted]")]
pub struct SessionToken(String);

impl SessionToken {
    /// Create a new instance from arbitrary text.
    pub fn new(value: impl Into<String>) -> Self {
        SessionToken(value.into())
    }

    /// View the interior value of the token.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Generate a new session token.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        SessionToken::new(hex::encode(bytes))
    }
}

impl AsRef<SessionToken> for SessionToken {
    fn as_ref(&self) -> &SessionToken {
        self
    }
}

/// An OAuth exchange code for the two-step authentication flow.
///
/// After a successful OAuth callback, Courier issues a short-lived, single-use
/// exchange code instead of returning a session token directly. The dashboard
/// backend then exchanges this code for a session token server-to-server.
///
/// Exchange codes are:
/// - High entropy (192 bits)
/// - Short-lived (60 seconds)
/// - Single-use (can only be redeemed once)
///
/// This avoids returning session tokens in URLs where they might be logged or
/// leaked.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Display, Deserialize, Serialize)]
#[debug("[redacted]")]
#[display("[redacted]")]
pub struct AuthCode(String);

impl AuthCode {
    /// Create a new instance from arbitrary text.
    pub fn new(value: impl Into<String>) -> Self {
        AuthCode(value.into())
    }

    /// View the interior value of the code.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Generate a new auth code.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut bytes);
        AuthCode::new(hex::encode(bytes))
    }
}

impl AsRef<AuthCode> for AuthCode {
    fn as_ref(&self) -> &AuthCode {
        self
    }
}

/// An authenticated token, which has been validated against the database.
///
/// This type can be extracted directly from a request using Axum's extractor
/// system. It will automatically validate the bearer token from the
/// Authorization header against the database before the handler is called.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
pub struct AuthenticatedToken {
    /// The account ID in the database.
    pub account_id: AccountId,

    /// The organization ID this API key is scoped to.
    pub org_id: OrgId,

    /// The plaintext value of the token for the user.
    pub plaintext: RawToken,
}

impl AsRef<RawToken> for AuthenticatedToken {
    fn as_ref(&self) -> &RawToken {
        &self.plaintext
    }
}

impl AsRef<AuthenticatedToken> for AuthenticatedToken {
    fn as_ref(&self) -> &AuthenticatedToken {
        self
    }
}

impl From<&AuthenticatedToken> for AuthenticatedToken {
    fn from(token: &AuthenticatedToken) -> Self {
        token.clone()
    }
}

impl FromRequestParts<api::State> for AuthenticatedToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &api::State,
    ) -> Result<Self, Self::Rejection> {
        let token = {
            let Some(header) = parts.headers.get(AUTHORIZATION) else {
                return Err((StatusCode::UNAUTHORIZED, "Authorization header required"));
            };
            let Ok(header) = header.to_str() else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Authorization header must be UTF8 encoded",
                ));
            };

            let header = match header.strip_prefix("Bearer") {
                Some(header) => header.trim(),
                None => header.trim(),
            };
            if header.is_empty() {
                return Err((StatusCode::BAD_REQUEST, "Provided token must not be empty"));
            }

            RawToken::new(header)
        };

        let Dep(db) = Dep::<db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Check out database connection",
                )
            })?;

        match db.validate(token).await {
            Ok(Some(auth)) => Ok(auth),
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "Invalid or revoked token")),
            Err(_) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error during authentication",
            )),
        }
    }
}

/// Session context for web UI authentication.
///
/// This represents an authenticated user session from the OAuth flow.
/// Unlike [`AuthenticatedToken`] which is tied to a specific organization via
/// API keys, session context identifies only the account. Organization context
/// must be provided in the URL for session-based requests.
///
/// This type can be extracted from requests using Axum's extractor system.
/// It validates the bearer token from the Authorization header against the
/// user_session table before the handler is called.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
pub struct SessionContext {
    /// The account ID of the authenticated user.
    pub account_id: AccountId,

    /// The session token (kept for potential refresh/invalidation).
    pub session_token: SessionToken,
}

impl FromRequestParts<api::State> for SessionContext {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &api::State,
    ) -> Result<Self, Self::Rejection> {
        let token = {
            let Some(header) = parts.headers.get(AUTHORIZATION) else {
                return Err((StatusCode::UNAUTHORIZED, "Authorization header required"));
            };
            let Ok(header) = header.to_str() else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Authorization header must be UTF8 encoded",
                ));
            };

            let header = match header.strip_prefix("Bearer") {
                Some(header) => header.trim(),
                None => header.trim(),
            };
            if header.is_empty() {
                return Err((StatusCode::BAD_REQUEST, "Provided token must not be empty"));
            }

            SessionToken::new(header)
        };

        let Dep(db) = Dep::<db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Check out database connection",
                )
            })?;

        match db.validate_session(&token).await {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "Invalid or expired session")),
            Err(_) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error during authentication",
            )),
        }
    }
}

/// Centralized API error type for consistent error responses.
///
/// This enum provides a unified way to handle common API errors across all
/// handlers, reducing duplication and ensuring consistent HTTP responses.
#[derive(Debug)]
pub enum ApiError {
    /// The user is not authorized to perform this action.
    Forbidden(&'static str),

    /// The requested resource was not found.
    NotFound(&'static str),

    /// The request was malformed or invalid.
    BadRequest(&'static str),

    /// An internal server error occurred.
    Internal(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Forbidden(msg) => write!(f, "Forbidden: {}", msg),
            ApiError::NotFound(msg) => write!(f, "Not found: {}", msg),
            ApiError::BadRequest(msg) => write!(f, "Bad request: {}", msg),
            ApiError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for ApiError {}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        match self {
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg).into_response(),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg).into_response(),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}

/// A session token with verified admin role for a specific organization.
///
/// This is an Axum extractor that validates the session token AND verifies
/// admin privileges before the handler runs. The handler is never called
/// if validation fails.
///
/// For session-based auth, the org_id must be provided separately (from URL
/// path) since sessions are account-scoped, not org-scoped.
#[derive(Clone, Debug)]
pub struct SessionTokenAdmin {
    /// The underlying session context.
    pub session: SessionContext,

    /// The account ID of the authenticated admin.
    pub account_id: AccountId,

    /// The organization ID where the user has admin privileges.
    pub org_id: OrgId,
}

/// A session token with verified membership for a specific organization.
///
/// This is an Axum extractor that validates the session token AND verifies
/// membership before the handler runs. The handler is never called if
/// validation fails.
///
/// For session-based auth, the org_id must be provided separately (from URL
/// path) since sessions are account-scoped, not org-scoped.
#[derive(Clone, Debug)]
pub struct SessionTokenMember {
    /// The underlying session context.
    pub session: SessionContext,

    /// The account ID of the authenticated member.
    pub account_id: AccountId,

    /// The organization ID where the user has membership.
    pub org_id: OrgId,

    /// The user's role in the organization.
    pub role: OrgRole,
}

impl SessionTokenAdmin {
    /// Get the organization ID.
    pub fn org_id(&self) -> OrgId {
        self.org_id
    }

    /// Get the account ID.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }
}

impl SessionTokenMember {
    /// Get the organization ID.
    pub fn org_id(&self) -> OrgId {
        self.org_id
    }

    /// Get the account ID.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Check if this member has admin privileges.
    pub fn is_admin(&self) -> bool {
        self.role.is_admin()
    }
}

/// Admin can be converted to Member since all admins are also members.
impl From<SessionTokenAdmin> for SessionTokenMember {
    fn from(admin: SessionTokenAdmin) -> Self {
        SessionTokenMember {
            session: admin.session,
            account_id: admin.account_id,
            org_id: admin.org_id,
            role: OrgRole::Admin,
        }
    }
}

/// Path parameter for extracting org_id from URL.
#[derive(Debug, Deserialize)]
struct OrgIdPath {
    org_id: i64,
}

impl FromRequestParts<api::State> for SessionTokenAdmin {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &api::State,
    ) -> Result<Self, Self::Rejection> {
        // First, extract the base SessionContext
        let session = SessionContext::from_request_parts(parts, state).await?;

        // Extract org_id from path
        let axum::extract::Path(OrgIdPath { org_id }) =
            axum::extract::Path::<OrgIdPath>::from_request_parts(parts, state)
                .await
                .map_err(|_| (StatusCode::BAD_REQUEST, "Missing org_id path parameter"))?;
        let org_id = OrgId::from_i64(org_id);

        // Get database connection
        let Dep(db) = Dep::<db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database connection error",
                )
            })?;

        // Verify admin role
        match db.get_member_role(org_id, session.account_id).await {
            Ok(Some(role)) if role.is_admin() => Ok(SessionTokenAdmin {
                account_id: session.account_id,
                org_id,
                session,
            }),
            Ok(Some(_)) => Err((StatusCode::FORBIDDEN, "Admin access required")),
            Ok(None) => Err((StatusCode::FORBIDDEN, "Not a member of this organization")),
            Err(e) => {
                tracing::error!(?e, "Failed to check member role");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error during authorization",
                ))
            }
        }
    }
}

impl FromRequestParts<api::State> for SessionTokenMember {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &api::State,
    ) -> Result<Self, Self::Rejection> {
        // First, extract the base SessionContext
        let session = SessionContext::from_request_parts(parts, state).await?;

        // Extract org_id from path
        let axum::extract::Path(OrgIdPath { org_id }) =
            axum::extract::Path::<OrgIdPath>::from_request_parts(parts, state)
                .await
                .map_err(|_| (StatusCode::BAD_REQUEST, "Missing org_id path parameter"))?;
        let org_id = OrgId::from_i64(org_id);

        // Get database connection
        let Dep(db) = Dep::<db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database connection error",
                )
            })?;

        // Verify membership
        match db.get_member_role(org_id, session.account_id).await {
            Ok(Some(role)) => Ok(SessionTokenMember {
                account_id: session.account_id,
                org_id,
                role,
                session,
            }),
            Ok(None) => Err((StatusCode::FORBIDDEN, "Not a member of this organization")),
            Err(e) => {
                tracing::error!(?e, "Failed to check member role");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error during authorization",
                ))
            }
        }
    }
}

/// An API key with verified admin role for the organization.
///
/// This is an Axum extractor that validates the API key AND verifies
/// admin privileges before the handler runs. The handler is never called
/// if validation fails.
///
/// API keys are already scoped to an organization, so the org_id is
/// embedded in the token.
#[derive(Clone, Debug)]
pub struct ApiKeyAdmin {
    /// The underlying authenticated token.
    pub token: AuthenticatedToken,

    /// The account ID of the authenticated admin.
    pub account_id: AccountId,

    /// The organization ID where the user has admin privileges.
    pub org_id: OrgId,
}

/// An API key with verified membership for the organization.
///
/// This is an Axum extractor that validates the API key AND verifies
/// membership before the handler runs. The handler is never called if
/// validation fails.
///
/// API keys are already scoped to an organization, so the org_id is
/// embedded in the token.
#[derive(Clone, Debug)]
pub struct ApiKeyMember {
    /// The underlying authenticated token.
    pub token: AuthenticatedToken,

    /// The account ID of the authenticated member.
    pub account_id: AccountId,

    /// The organization ID where the user has membership.
    pub org_id: OrgId,

    /// The user's role in the organization.
    pub role: OrgRole,
}

impl ApiKeyAdmin {
    /// Get the organization ID.
    pub fn org_id(&self) -> OrgId {
        self.org_id
    }

    /// Get the account ID.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }
}

impl ApiKeyMember {
    /// Get the organization ID.
    pub fn org_id(&self) -> OrgId {
        self.org_id
    }

    /// Get the account ID.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Check if this member has admin privileges.
    pub fn is_admin(&self) -> bool {
        self.role.is_admin()
    }
}

/// Admin can be converted to Member since all admins are also members.
impl From<ApiKeyAdmin> for ApiKeyMember {
    fn from(admin: ApiKeyAdmin) -> Self {
        ApiKeyMember {
            token: admin.token,
            account_id: admin.account_id,
            org_id: admin.org_id,
            role: OrgRole::Admin,
        }
    }
}

impl FromRequestParts<api::State> for ApiKeyAdmin {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &api::State,
    ) -> Result<Self, Self::Rejection> {
        // First, extract the base AuthenticatedToken
        let token = AuthenticatedToken::from_request_parts(parts, state).await?;

        // Get database connection
        let Dep(db) = Dep::<db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database connection error",
                )
            })?;

        // Verify admin role
        match db.get_member_role(token.org_id, token.account_id).await {
            Ok(Some(role)) if role.is_admin() => Ok(ApiKeyAdmin {
                account_id: token.account_id,
                org_id: token.org_id,
                token,
            }),
            Ok(Some(_)) => Err((StatusCode::FORBIDDEN, "Admin access required")),
            Ok(None) => Err((StatusCode::FORBIDDEN, "Not a member of this organization")),
            Err(e) => {
                tracing::error!(?e, "Failed to check member role");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error during authorization",
                ))
            }
        }
    }
}

impl FromRequestParts<api::State> for ApiKeyMember {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &api::State,
    ) -> Result<Self, Self::Rejection> {
        // First, extract the base AuthenticatedToken
        let token = AuthenticatedToken::from_request_parts(parts, state).await?;

        // Get database connection
        let Dep(db) = Dep::<db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database connection error",
                )
            })?;

        // Verify membership
        match db.get_member_role(token.org_id, token.account_id).await {
            Ok(Some(role)) => Ok(ApiKeyMember {
                account_id: token.account_id,
                org_id: token.org_id,
                role,
                token,
            }),
            Ok(None) => Err((StatusCode::FORBIDDEN, "Not a member of this organization")),
            Err(e) => {
                tracing::error!(?e, "Failed to check member role");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error during authorization",
                ))
            }
        }
    }
}

/// Legacy type aliases for backward compatibility during migration.
/// These will be removed once all handlers are updated.
pub type SessionAdmin = SessionTokenAdmin;
pub type SessionMember = SessionTokenMember;

impl SessionContext {
    /// Verify that the user has admin privileges in the specified organization.
    ///
    /// Returns a [`SessionTokenAdmin`] if the user is an admin, or an
    /// [`ApiError`] if they are not authorized.
    ///
    /// # Deprecated
    ///
    /// Prefer using `SessionTokenAdmin` as an extractor directly in handler
    /// signatures. This method is provided for backward compatibility
    /// during migration.
    pub async fn try_admin(
        &self,
        db: &db::Postgres,
        org_id: OrgId,
    ) -> Result<SessionTokenAdmin, ApiError> {
        match db.get_member_role(org_id, self.account_id).await {
            Ok(Some(role)) if role.is_admin() => Ok(SessionTokenAdmin {
                session: self.clone(),
                account_id: self.account_id,
                org_id,
            }),
            Ok(Some(_)) => Err(ApiError::Forbidden("Admin access required")),
            Ok(None) => Err(ApiError::Forbidden("Not a member of this organization")),
            Err(e) => {
                tracing::error!(?e, "Failed to check member role");
                Err(ApiError::Internal(e.to_string()))
            }
        }
    }

    /// Verify that the user is a member of the specified organization.
    ///
    /// Returns a [`SessionTokenMember`] if the user is a member (with any
    /// role), or an [`ApiError`] if they are not authorized.
    ///
    /// # Deprecated
    ///
    /// Prefer using `SessionTokenMember` as an extractor directly in handler
    /// signatures. This method is provided for backward compatibility
    /// during migration.
    pub async fn try_member(
        &self,
        db: &db::Postgres,
        org_id: OrgId,
    ) -> Result<SessionTokenMember, ApiError> {
        match db.get_member_role(org_id, self.account_id).await {
            Ok(Some(role)) => Ok(SessionTokenMember {
                session: self.clone(),
                account_id: self.account_id,
                org_id,
                role,
            }),
            Ok(None) => Err(ApiError::Forbidden("Not a member of this organization")),
            Err(e) => {
                tracing::error!(?e, "Failed to check member role");
                Err(ApiError::Internal(e.to_string()))
            }
        }
    }
}
