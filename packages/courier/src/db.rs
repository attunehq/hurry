//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use std::collections::{HashMap, HashSet};

use clients::courier::v1::{
    GlibcVersion, Key, SavedUnit, SavedUnitHash,
    cache::{CargoRestoreRequest, CargoSaveRequest},
};
use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use derive_more::Debug;
use futures::StreamExt;
use sqlx::migrate::Migrate;
use sqlx::{PgPool, migrate::Migrator};
use tracing::debug;

use crate::{
    auth::{AccountId, AuthenticatedToken, OrgId, RawToken},
    crypto::TokenHash,
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

    /// Validate that all migrations have been applied to the database.
    ///
    /// This checks that:
    /// 1. All migrations in the codebase have been applied
    /// 2. Applied migrations have matching checksums (no modified migrations)
    /// 3. No migrations exist in the database that are missing from the
    ///    codebase (unless `ignore_missing` is set in the MIGRATOR)
    ///
    /// This is intended for use at server startup to ensure the database schema
    /// is up-to-date before serving traffic. It does NOT apply migrations;
    /// use the separate `migrate` command for that.
    #[tracing::instrument(name = "Postgres::validate_migrations")]
    pub async fn validate_migrations(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await.context("acquire connection")?;

        conn.ensure_migrations_table()
            .await
            .context("ensure migrations table")?;

        // A dirty migration is one that failed partway through application.
        if let Some(version) = conn.dirty_version().await.context("check dirty version")? {
            bail!(
                "Database has a dirty migration (version {version}). \
                 A previous migration failed partway through. \
                 Manually resolve the issue and re-run 'courier migrate'."
            );
        }

        let applied = conn
            .list_applied_migrations()
            .await
            .context("list applied migrations")?;
        let applied_map = applied
            .iter()
            .map(|m| (m.version, m))
            .collect::<HashMap<_, _>>();
        let applied_versions = applied.iter().map(|m| m.version).collect::<HashSet<_>>();

        // Expected migrations are the up-migrations defined in the codebase.
        let expected = Self::MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_up_migration())
            .collect::<Vec<_>>();
        let expected_versions = expected.iter().map(|m| m.version).collect::<HashSet<_>>();

        // Pending migrations are in the codebase but not yet applied to the database.
        let mut pending = expected
            .iter()
            .filter(|m| !applied_versions.contains(&m.version))
            .map(|m| m.version)
            .collect::<Vec<_>>();
        pending.sort();
        if !pending.is_empty() {
            let versions = pending
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Database has pending migrations: [{versions}]. \
                 Run 'courier migrate' first."
            );
        }

        // Checksum mismatches occur when a migration file was modified after being
        // applied.
        let mismatched = expected
            .iter()
            .filter_map(|m| {
                applied_map.get(&m.version).and_then(|applied| {
                    if m.checksum != applied.checksum {
                        Some(m.version)
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();
        if !mismatched.is_empty() {
            let versions = mismatched
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Database has migrations with checksum mismatches: [{versions}]. \
                 Migrations were modified after being applied. \
                 This likely indicates a development error."
            );
        }

        // Missing migrations are applied to the database but not present in the
        // codebase.
        if !Self::MIGRATOR.ignore_missing {
            let mut missing = applied_versions
                .difference(&expected_versions)
                .copied()
                .collect::<Vec<_>>();
            missing.sort();
            if !missing.is_empty() {
                let versions = missing
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "Database has applied migrations missing from codebase: [{versions}]. \
                     This may indicate you're running an older version of the code."
                );
            }
        }

        Ok(())
    }
}

impl AsRef<PgPool> for Postgres {
    fn as_ref(&self) -> &PgPool {
        &self.pool
    }
}

impl Postgres {
    #[tracing::instrument(name = "Postgres::save_cargo_cache")]
    pub async fn cargo_cache_save(
        &self,
        auth: &AuthenticatedToken,
        request: CargoSaveRequest,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // TODO: bulk insert
        for item in request {
            let data = serde_json::to_value(&item.unit)
                .with_context(|| format!("serialize data to json: {:?}", item.unit))?;
            sqlx::query!(
                r#"INSERT INTO cargo_saved_unit (organization_id, unit_hash, unit_resolved_target, linux_glibc_version, data)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT DO NOTHING"#,
                auth.org_id.as_i64(),
                item.unit.unit_hash().as_str(),
                item.resolved_target,
                item.linux_glibc_version.map(|v| v.to_string()),
                data,
            )
            .execute(tx.as_mut())
            .await
            .context("insert serialized cache data")?;
        }

        tx.commit().await.context("commit transaction")
    }

    #[tracing::instrument(name = "Postgres::cargo_cache_restore")]
    pub async fn cargo_cache_restore(
        &self,
        auth: &AuthenticatedToken,
        request: CargoRestoreRequest,
    ) -> Result<HashMap<SavedUnitHash, SavedUnit>> {
        let mut rows = sqlx::query!(
            r#"SELECT unit_hash, linux_glibc_version, data
            FROM cargo_saved_unit
            WHERE organization_id = $1
            AND unit_hash = ANY($2)"#,
            auth.org_id.as_i64(),
            &request
                .units
                .iter()
                .cloned()
                .map(|h| h.to_string())
                .collect::<Vec<_>>(),
        )
        .fetch(&self.pool);

        let mut artifacts = HashMap::with_capacity(request.units.len());
        while let Some(row) = rows.next().await {
            let row = row.context("read rows")?;

            let key = row.unit_hash.into();
            let unit = serde_json::from_value::<SavedUnit>(row.data)
                .with_context(|| format!("deserialize value for cache key: {}", key))?;

            // Check for glibc version compatibility for units that compile
            // against glibc.
            if let Some(ref host_glibc) = request.host_glibc_version {
                let Some(saved_glibc_string) = row.linux_glibc_version else {
                    // Skip units without glibc version info. Note that this
                    // should never happen, since all units with a matching unit
                    // hash will all be on the same target, and all units of a
                    // target either do or do not have glibc version info.
                    continue;
                };
                let saved_glibc = saved_glibc_string.as_str().parse::<GlibcVersion>()?;
                if *host_glibc < saved_glibc {
                    // Skip units with incompatible glibc versions.
                    continue;
                }
            } else if let Some(_) = row.linux_glibc_version {
                // Skip units that have glibc version info when host doesn't
                // have glibc version info (i.e., non-linux targets). This is
                // another thing that should never happen.
                continue;
            }

            artifacts.insert(key, unit);
        }

        Ok(artifacts)
    }

    /// Lookup account and org for a raw token by direct hash comparison.
    #[tracing::instrument(name = "Postgres::token_lookup", skip(token))]
    async fn token_lookup(
        &self,
        token: impl AsRef<RawToken>,
    ) -> Result<Option<(AccountId, OrgId)>> {
        let hash = TokenHash::new(token.as_ref().expose());
        let row = sqlx::query!(
            r#"
            SELECT
                account.id as account_id,
                account.organization_id
            FROM api_key
            JOIN account ON api_key.account_id = account.id
            WHERE api_key.hash = $1 AND api_key.revoked_at IS NULL
            "#,
            hash.as_bytes(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("query for token")?;

        Ok(row.map(|r| {
            (
                AccountId::from_i64(r.account_id),
                OrgId::from_i64(r.organization_id),
            )
        }))
    }

    /// Validate a raw token against the database.
    ///
    /// Returns `Some(AuthenticatedToken)` if the token is valid and not
    /// revoked, otherwise returns `None`. Errors are only returned for
    /// database failures.
    #[tracing::instrument(name = "Postgres::validate", skip(token))]
    pub async fn validate(&self, token: impl Into<RawToken>) -> Result<Option<AuthenticatedToken>> {
        let token = token.into();
        Ok(self
            .token_lookup(&token)
            .await?
            .map(|(account_id, org_id)| AuthenticatedToken {
                account_id,
                org_id,
                plaintext: token,
            }))
    }

    /// Generate a new token for the account in the database.
    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[allow(dead_code)]
    #[tracing::instrument(name = "Postgres::create_token")]
    pub async fn create_token(&self, account: AccountId, name: &str) -> Result<RawToken> {
        use rand::RngCore;

        let plaintext = {
            let mut plaintext = [0u8; 16];
            rand::thread_rng()
                .try_fill_bytes(&mut plaintext)
                .context("generate plaintext key")?;
            hex::encode(plaintext)
        };

        let token = TokenHash::new(&plaintext);
        sqlx::query!(
            r#"
            INSERT INTO api_key (account_id, name, hash)
            VALUES ($1, $2, $3)
            "#,
            account.as_i64(),
            name,
            token.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("insert token")?;

        Ok(RawToken::new(plaintext))
    }

    /// Revoke the specified token.
    /// Currently only used in tests. If used elsewhere, feel free to make this
    /// generally available.
    #[allow(dead_code)]
    #[tracing::instrument(name = "Postgres::revoke_token", skip(token))]
    pub async fn revoke_token(&self, token: impl AsRef<RawToken>) -> Result<()> {
        let hash = TokenHash::new(token.as_ref().expose());

        let results = sqlx::query!(
            r#"
            UPDATE api_key
            SET revoked_at = now()
            WHERE hash = $1
            "#,
            hash.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("revoke token")?;

        if results.rows_affected() == 0 {
            bail!("no such token to revoke in the database");
        }

        Ok(())
    }

    /// Grant an organization access to a CAS key.
    ///
    /// This is idempotent: if the organization already has access, this is a
    /// no-op.
    ///
    /// Returns `true` if access was newly granted, `false` if the org already
    /// had access.
    #[tracing::instrument(name = "Postgres::grant_cas_access")]
    pub async fn grant_cas_access(&self, org_id: OrgId, key: &Key) -> Result<bool> {
        // First, ensure the CAS key exists
        let key_id = sqlx::query!(
            r#"
            INSERT INTO cas_key (content)
            VALUES ($1)
            ON CONFLICT (content) DO UPDATE SET content = EXCLUDED.content
            RETURNING id
            "#,
            key.as_bytes(),
        )
        .fetch_one(&self.pool)
        .await
        .context("upsert cas key")?
        .id;

        // Then grant access to the organization
        let result = sqlx::query!(
            r#"
            INSERT INTO cas_access (organization_id, cas_key_id)
            VALUES ($1, $2)
            ON CONFLICT (organization_id, cas_key_id) DO NOTHING
            "#,
            org_id.as_i64(),
            key_id,
        )
        .execute(&self.pool)
        .await
        .context("grant org access to cas key")?;

        // If rows_affected is 1, we inserted a new row (newly granted access)
        // If rows_affected is 0, the row already existed (org already had access)
        Ok(result.rows_affected() == 1)
    }

    /// Check if an organization has access to a CAS key.
    #[tracing::instrument(name = "Postgres::check_cas_access")]
    pub async fn check_cas_access(&self, org_id: OrgId, key: &Key) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM cas_access
                WHERE organization_id = $1
                AND cas_key_id = (SELECT id FROM cas_key WHERE content = $2)
            ) as "exists!"
            "#,
            org_id.as_i64(),
            key.as_bytes(),
        )
        .fetch_one(&self.pool)
        .await
        .context("check cas access")?;

        Ok(result.exists)
    }

    /// Check which keys from a set the organization has access to.
    /// Returns a HashSet of keys that the organization can access.
    #[tracing::instrument(name = "Postgres::check_cas_access_bulk", skip(keys))]
    pub async fn check_cas_access_bulk(
        &self,
        org_id: OrgId,
        keys: &[Key],
    ) -> Result<std::collections::HashSet<Key>> {
        if keys.is_empty() {
            return Ok(std::collections::HashSet::new());
        }

        let key_bytes: Vec<Vec<u8>> = keys.iter().map(|k| k.as_bytes().to_vec()).collect();

        let rows = sqlx::query!(
            r#"
            SELECT cas_key.content
            FROM cas_key
            JOIN cas_access ON cas_key.id = cas_access.cas_key_id
            WHERE cas_access.organization_id = $1
            AND cas_key.content = ANY($2)
            "#,
            org_id.as_i64(),
            &key_bytes,
        )
        .fetch_all(&self.pool)
        .await
        .context("check cas access bulk")?;

        rows.into_iter()
            .map(|row| {
                Key::from_bytes(&row.content)
                    .with_context(|| format!("parse key: {:x?}", &row.content))
            })
            .collect()
    }

    #[tracing::instrument(name = "Postgres::cargo_cache_reset")]
    pub async fn cargo_cache_reset(&self, auth: &AuthenticatedToken) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query!(
            "delete from cargo_saved_unit where organization_id = $1",
            auth.org_id.as_i64()
        )
        .execute(tx.as_mut())
        .await
        .context("delete saved units")?;

        sqlx::query!(
            "delete from cas_access where organization_id = $1",
            auth.org_id.as_i64()
        )
        .execute(tx.as_mut())
        .await
        .context("delete cas access")?;

        tx.commit().await?;
        Ok(())
    }
}
