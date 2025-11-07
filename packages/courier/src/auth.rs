use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use derive_more::{Debug, Display, From, Into};
use serde::{Deserialize, Serialize};

/// An ID uniquely identifying an organization.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Debug,
    Display,
    Default,
    Deserialize,
    Serialize,
)]
pub struct OrgId(u64);

impl OrgId {
    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    #[cfg(test)]
    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }

    pub fn from_i64(id: i64) -> Self {
        Self(id as u64)
    }
}

/// An ID uniquely identifying an account.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Debug,
    Display,
    Default,
    Deserialize,
    Serialize,
    From,
    Into,
)]
pub struct AccountId(u64);

impl AccountId {
    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn from_i64(id: i64) -> Self {
        Self(id as u64)
    }

    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

/// An unauthenticated token extracted from the Authorization header.
///
/// These are provided by the client and have not yet been validated against
/// the database. To validate a token, use [`crate::db::Postgres::validate()`].
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[debug("RawToken(..)")]
pub struct RawToken(String);

impl RawToken {
    /// Create a new raw token.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// View the token as a string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<S: Into<String>> From<S> for RawToken {
    fn from(token: S) -> Self {
        Self::new(token)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for RawToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(header) = parts.headers.get(AUTHORIZATION) else {
            return Err((StatusCode::UNAUTHORIZED, "Authorization header required"));
        };
        let Ok(token) = header.to_str() else {
            return Err((
                StatusCode::BAD_REQUEST,
                "Authorization header must be a string",
            ));
        };

        let token = match token.strip_prefix("Bearer") {
            Some(token) => token.trim(),
            None => token.trim(),
        };
        if token.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Empty authorization token"));
        }

        Ok(RawToken::new(token))
    }
}

/// An authenticated token, which has been validated against the database.
///
/// This type cannot be extracted directly from a request; it must be obtained
/// by calling [`crate::db::Postgres::validate()`] with a [`RawToken`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthenticatedToken {
    /// The account ID in the database.
    pub account_id: AccountId,

    /// The organization ID in the database.
    pub org_id: OrgId,

    /// The token that was authenticated.
    pub token: RawToken,
}

impl From<AuthenticatedToken> for RawToken {
    fn from(val: AuthenticatedToken) -> Self {
        val.token
    }
}

impl AsRef<RawToken> for AuthenticatedToken {
    fn as_ref(&self) -> &RawToken {
        &self.token
    }
}
