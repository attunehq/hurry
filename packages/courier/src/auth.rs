use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use color_eyre::Result;
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
    From,
    Into,
)]
pub struct OrgId(u64);

impl<S: Send + Sync> FromRequestParts<S> for OrgId {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        const ORG_ID_HEADER: &str = "x-org-id";
        let Some(header) = parts.headers.get(ORG_ID_HEADER) else {
            return Err((
                StatusCode::UNAUTHORIZED,
                const_str::format!("{ORG_ID_HEADER} header required"),
            ));
        };
        let Ok(header) = header.to_str() else {
            return Err((
                StatusCode::BAD_REQUEST,
                const_str::format!("{ORG_ID_HEADER} header must be a string"),
            ));
        };

        let Ok(parsed) = header.trim().parse::<u64>() else {
            return Err((
                StatusCode::BAD_REQUEST,
                const_str::format!("{ORG_ID_HEADER} header must be a valid unsigned number"),
            ));
        };

        Ok(OrgId::from(parsed))
    }
}

/// An ID uniquely identifying a user.
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
pub struct UserId(u64);

/// An authenticated token, which has been validated.
#[derive(Debug, Deserialize, Serialize)]
pub struct AuthenticatedToken {
    /// The user ID in the database.
    pub user_id: UserId,

    /// The organization ID in the database.
    pub org_id: OrgId,

    /// The token that was authenticated.
    pub token: RawToken,
}

impl Into<RawToken> for AuthenticatedToken {
    fn into(self) -> RawToken {
        self.token
    }
}

impl AsRef<RawToken> for AuthenticatedToken {
    fn as_ref(&self) -> &RawToken {
        &self.token
    }
}

/// An unauthenticated token.
///
/// These are provided by the client and have not yet been validated.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[debug("RawToken(..)")]
pub struct RawToken(String);

impl RawToken {
    /// Create a new raw token.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// View the token as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
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
