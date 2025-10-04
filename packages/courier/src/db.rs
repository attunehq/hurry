//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use std::collections::HashSet;

use color_eyre::{Result, eyre::Context};
use derive_more::Debug;
use futures::StreamExt;
use sqlx::{PgPool, migrate::Migrator};

use crate::{
    auth::{AuthenticatedToken, OrgId, RawToken, UserId},
    storage::Key,
};

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
    ///
    /// If the token is valid, returns the authenticated token. If the token is
    /// invalid, returns `None`; errors are only returned if there is an error
    /// communicating with the database.
    pub async fn validate(
        &self,
        org_id: OrgId,
        token: RawToken,
    ) -> Result<Option<AuthenticatedToken>> {
        sqlx::query!(
            "select users.id
            from users
            join api_keys on users.id = api_keys.user_id
            where users.organization_id = $1
            and api_keys.content = $2",
            org_id.as_i64(),
            token.as_bytes(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch user id for token")
        .map(|query| {
            query.map(|row| AuthenticatedToken {
                user_id: UserId::from_i64(row.id),
                org_id,
                token,
            })
        })
    }

    /// Check if the given user has access to the given CAS key.
    pub async fn user_has_cas_key(&self, user_id: UserId, key: &Key) -> Result<bool> {
        sqlx::query!(
            "select exists(
            select 1 from cas_access
            join users on cas_access.org_id = users.organization_id
            where users.id = $1
            and cas_access.cas_key_id = (select id from cas_keys where content = $2))",
            user_id.as_i64(),
            key.as_bytes(),
        )
        .fetch_one(&self.pool)
        .await
        .context("fetch user has cas key")
        .map(|query| query.exists.unwrap_or(false))
    }

    /// Get the allowed CAS keys for the given user.
    ///
    /// Today this just returns all CAS keys, but in the future we plan to
    /// select the top N most frequently accessed keys.
    pub async fn user_allowed_cas_keys(&self, user_id: UserId, limit: u64) -> Result<HashSet<Key>> {
        // TODO: use frequency_user_cas_key once it's implemented
        let mut rows = sqlx::query!(
            "select cas_keys.content
            from cas_keys
            join cas_access on cas_keys.id = cas_access.cas_key_id
            join users on cas_access.org_id = users.organization_id
            where users.id = $1
            limit $2",
            user_id.as_i64(),
            limit as i64,
        )
        .fetch(&self.pool);

        let mut keys = HashSet::new();
        while let Some(row) = rows.next().await {
            let row = row.context("read row")?;
            let key = Key::from(row.content);
            keys.insert(key);
        }
        Ok(keys)
    }
}

impl AsRef<PgPool> for Postgres {
    fn as_ref(&self) -> &PgPool {
        &self.pool
    }
}
