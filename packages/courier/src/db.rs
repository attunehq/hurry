//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use color_eyre::{Result, eyre::Context};
use derive_more::{Debug, Display};
use sqlx::{PgPool, Type, migrate::Migrator};
use tap::Conv;

use crate::auth::{AuthenticatedToken, RawToken};

/// A connected Postgres database instance.
#[derive(Clone, Debug)]
pub struct Postgres {
    pool: PgPool,
}

impl Postgres {
    /// The migrator for the database.
    pub const MIGRATOR: Migrator = sqlx::migrate!("./schema/migrations");

    /// Connect to the Postgres database.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPool::connect(url).await?;
        Ok(Self { pool })
    }

    /// Validate the provided raw token in the database.
    pub async fn validate(
        &self,
        org_id: InternalOrgId,
        token: RawToken,
    ) -> Result<AuthenticatedToken> {
        let row = sqlx::query!(
            "SELECT users.id
            FROM users
            JOIN api_keys ON users.id = api_keys.user_id
            WHERE users.organization_id = $1
            AND api_keys.content = $2",
            org_id.0,
            token.as_bytes(),
        )
        .fetch_one(&self.pool)
        .await
        .context("fetch user id for token")?;
        Ok(AuthenticatedToken {
            user_id: InternalUserId(row.id).into(),
            org_id: org_id.into(),
            token,
        })
    }
}

impl AsRef<PgPool> for Postgres {
    fn as_ref(&self) -> &PgPool {
        &self.pool
    }
}

/// Internal representation of an organization ID.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default, Type)]
pub struct InternalOrgId(i64);

impl From<crate::auth::OrgId> for InternalOrgId {
    fn from(id: crate::auth::OrgId) -> Self {
        Self(id.conv::<u64>() as i64)
    }
}

impl From<InternalOrgId> for crate::auth::OrgId {
    fn from(InternalOrgId(id): InternalOrgId) -> Self {
        (id as u64).into()
    }
}

/// Internal representation of a user ID.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default, Type)]
pub struct InternalUserId(i64);

impl From<crate::auth::UserId> for InternalUserId {
    fn from(id: crate::auth::UserId) -> Self {
        Self(id.conv::<u64>() as i64)
    }
}

impl From<InternalUserId> for crate::auth::UserId {
    fn from(InternalUserId(id): InternalUserId) -> Self {
        (id as u64).into()
    }
}
