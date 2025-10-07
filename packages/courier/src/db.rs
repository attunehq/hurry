//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use std::collections::HashSet;

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use derive_more::Debug;
use futures::StreamExt;
use sqlx::{PgPool, migrate::Migrator};

use crate::{
    auth::{AuthenticatedToken, OrgId, RawToken, UserId},
    storage::Key,
};

/// A connected Postgres database instance.
#[derive(Clone, Debug)]
#[debug("Postgres(pool_size = {})", self.pool.size())]
pub struct Postgres {
    pub pool: PgPool,
}

impl Postgres {
    /// The migrator for the database.
    pub const MIGRATOR: Migrator = sqlx::migrate!("./schema/migrations");

    /// Connect to the Postgres database.
    #[tracing::instrument(name = "Postgres::connect")]
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPool::connect(url).await?;
        Ok(Self { pool })
    }

    /// Ping the database to ensure the connection is alive.
    #[tracing::instrument(name = "Postgres::ping")]
    pub async fn ping(&self) -> Result<()> {
        let row = sqlx::query!("select 1 as pong")
            .fetch_one(&self.pool)
            .await
            .context("ping database")?;
        if row.pong.is_none_or(|pong| pong != 1) {
            bail!("database ping failed; unexpected response: {row:?}");
        }
        Ok(())
    }

    /// Validate the provided raw token in the database.
    ///
    /// If the token is valid, returns the authenticated token. If the token is
    /// invalid, returns `None`; errors are only returned if there is an error
    /// communicating with the database.
    #[tracing::instrument(name = "Postgres::validate", skip(token))]
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
            token.as_str(),
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
    #[tracing::instrument(name = "Postgres::user_has_cas_key")]
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
    /// Returns the top N most frequently accessed keys for the user based on
    /// access patterns over the last 7 days.
    #[tracing::instrument(name = "Postgres::user_allowed_cas_keys")]
    pub async fn user_allowed_cas_keys(&self, user_id: UserId, limit: u64) -> Result<HashSet<Key>> {
        let mut rows = sqlx::query!(
            "select cas_keys.content, count(*) as freq
            from frequency_user_cas_key
            join cas_keys on frequency_user_cas_key.cas_key_id = cas_keys.id
            where frequency_user_cas_key.user_id = $1
            and frequency_user_cas_key.accessed > now() - interval '7 days'
            group by cas_keys.id, cas_keys.content
            order by freq desc
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

    /// Grant an organization access to a CAS key.
    ///
    /// This is idempotent: if the org already has access, this is a no-op.
    /// The key is automatically inserted into `cas_keys` if it doesn't exist.
    #[tracing::instrument(name = "Postgres::grant_org_cas_key")]
    pub async fn grant_org_cas_key(&self, org_id: OrgId, key: &Key) -> Result<()> {
        // We use a two-CTE approach to handle the "insert or get existing"
        // pattern in a single round trip without creating dead tuples:
        //
        // 1. `inserted` CTE: tries to insert the key, returns ID if successful
        // 2. `key_id` CTE: unions the insert result with a fallback SELECT that
        //    only runs if the insert returned nothing (due to conflict)
        // 3. Final INSERT: grants access using the key ID from step 2
        //
        // We avoid using `ON CONFLICT DO UPDATE` because that creates dead
        // tuples even when doing a no-op update, which increases vacuum
        // overhead.
        sqlx::query!(
            "with inserted as (
                insert into cas_keys (content)
                values ($2)
                on conflict (content) do nothing
                returning id
            ),
            key_id as (
                select id from inserted
                union all
                select id from cas_keys where content = $2
                limit 1
            )
            insert into cas_access (org_id, cas_key_id)
            select $1, id from key_id
            on conflict (org_id, cas_key_id) do nothing",
            org_id.as_i64(),
            key.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("grant org access to cas key")?;

        Ok(())
    }

    /// Record that a user accessed a CAS key.
    ///
    /// This is used for frequency tracking to preload hot keys into memory.
    #[tracing::instrument(name = "Postgres::record_cas_key_access")]
    pub async fn record_cas_key_access(&self, user_id: UserId, key: &Key) -> Result<()> {
        sqlx::query!(
            "insert into frequency_user_cas_key (user_id, cas_key_id)
            select $1, id from cas_keys where content = $2",
            user_id.as_i64(),
            key.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("record cas key access")?;

        Ok(())
    }
}

impl AsRef<PgPool> for Postgres {
    fn as_ref(&self) -> &PgPool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn open_test_database(pool: PgPool) {
        let db = crate::db::Postgres { pool };
        db.ping().await.expect("ping database");
    }
}
