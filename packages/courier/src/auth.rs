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
