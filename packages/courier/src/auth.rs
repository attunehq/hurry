use aerosol::axum::Dep;
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use derive_more::Display;
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
)]
pub struct AccountId(u64);

impl AccountId {
    pub fn from_i64(id: i64) -> Self {
        Self(id as u64)
    }
}

/// An authenticated token, which has been validated against the database.
///
/// This type can be extracted directly from a request using Axum's extractor
/// system. It will automatically validate the bearer token from the
/// Authorization header against the database before the handler is called.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthenticatedToken {
    /// The account ID in the database.
    pub account_id: AccountId,

    /// The organization ID in the database.
    pub org_id: OrgId,
}

impl FromRequestParts<crate::api::State> for AuthenticatedToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::api::State,
    ) -> Result<Self, Self::Rejection> {
        // Extract and parse Authorization header before borrowing parts mutably
        let token = {
            let Some(header) = parts.headers.get(AUTHORIZATION) else {
                return Err((StatusCode::UNAUTHORIZED, "Authorization header required"));
            };
            let Ok(token_str) = header.to_str() else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Authorization header must be a string",
                ));
            };

            let token = match token_str.strip_prefix("Bearer") {
                Some(token) => token.trim(),
                None => token_str.trim(),
            };
            if token.is_empty() {
                return Err((StatusCode::BAD_REQUEST, "Empty authorization token"));
            }

            token.to_string()
        };

        // Get database from state
        let Dep(db) = Dep::<crate::db::Postgres>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to extract database",
                )
            })?;

        // Validate token against database
        match db.validate(&token).await {
            Ok(Some(auth)) => Ok(auth),
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "Invalid or revoked token")),
            Err(_) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error during authentication",
            )),
        }
    }
}
