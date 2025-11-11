use aerosol::axum::Dep;
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use derive_more::{Debug, Display};
use serde::{Deserialize, Serialize};

use crate::{api, db};

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

    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[cfg(test)]
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

/// An authenticated token, which has been validated against the database.
///
/// This type can be extracted directly from a request using Axum's extractor
/// system. It will automatically validate the bearer token from the
/// Authorization header against the database before the handler is called.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
pub struct AuthenticatedToken {
    /// The account ID in the database.
    pub account_id: AccountId,

    /// The organization ID in the database.
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
