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
    eyre::{Context, bail, eyre},
};
use derive_more::Debug;
use futures::StreamExt;
use sqlx::migrate::Migrate;
use sqlx::{PgPool, migrate::Migrator};
use time::OffsetDateTime;
use tracing::debug;

use crate::{
    auth::{
        AccountId, ApiKeyId, AuthenticatedToken, InvitationId, OrgId, OrgRole, RawToken,
        SessionContext, SessionId, SessionToken,
    },
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
    pub async fn cargo_cache_save(&self, org_id: OrgId, request: CargoSaveRequest) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // TODO: bulk insert
        for item in request {
            let data = serde_json::to_value(&item.unit)
                .with_context(|| format!("serialize data to json: {:?}", item.unit))?;
            sqlx::query!(
                r#"INSERT INTO cargo_saved_unit (organization_id, unit_hash, unit_resolved_target, linux_glibc_version, data)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT DO NOTHING"#,
                org_id.as_i64(),
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
        org_id: OrgId,
        request: CargoRestoreRequest,
    ) -> Result<HashMap<SavedUnitHash, SavedUnit>> {
        let mut rows = sqlx::query!(
            r#"SELECT unit_hash, linux_glibc_version, data
            FROM cargo_saved_unit
            WHERE organization_id = $1
            AND unit_hash = ANY($2)"#,
            org_id.as_i64(),
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
    ) -> Result<Option<(AccountId, Option<OrgId>)>> {
        let hash = TokenHash::new(token.as_ref().expose());
        let row = sqlx::query!(
            r#"
            SELECT
                api_key.account_id,
                api_key.organization_id
            FROM api_key
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
                r.organization_id.map(OrgId::from_i64),
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
    pub async fn cargo_cache_reset(&self, org_id: OrgId) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query!(
            "delete from cargo_saved_unit where organization_id = $1",
            org_id.as_i64()
        )
        .execute(tx.as_mut())
        .await
        .context("delete saved units")?;

        sqlx::query!(
            "delete from cas_access where organization_id = $1",
            org_id.as_i64()
        )
        .execute(tx.as_mut())
        .await
        .context("delete cas access")?;

        tx.commit().await?;
        Ok(())
    }
}

// =============================================================================
// Account Operations
// =============================================================================

/// An account record from the database.
///
/// Note: Organization membership is tracked via the `organization_member`
/// table, not directly on the account. Use `list_organizations_for_account` to
/// get an account's organizations.
#[derive(Clone, Debug)]
pub struct Account {
    pub id: AccountId,
    pub email: String,
    pub name: Option<String>,
    pub disabled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

impl Postgres {
    /// Create a new account.
    ///
    /// Note: This only creates the account record. Use
    /// `add_organization_member` to associate the account with an
    /// organization.
    #[tracing::instrument(name = "Postgres::create_account")]
    pub async fn create_account(&self, email: &str, name: Option<&str>) -> Result<AccountId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO account (email, name)
            VALUES ($1, $2)
            RETURNING id
            "#,
            email,
            name,
        )
        .fetch_one(&self.pool)
        .await
        .context("insert account")?;

        Ok(AccountId::from_i64(row.id))
    }

    /// Get an account by ID.
    #[tracing::instrument(name = "Postgres::get_account")]
    pub async fn get_account(&self, account_id: AccountId) -> Result<Option<Account>> {
        let row = sqlx::query!(
            r#"
            SELECT id, email, name, disabled_at, created_at
            FROM account
            WHERE id = $1
            "#,
            account_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch account")?;

        Ok(row.map(|r| Account {
            id: AccountId::from_i64(r.id),
            email: r.email,
            name: r.name,
            disabled_at: r.disabled_at,
            created_at: r.created_at,
        }))
    }

    /// Get an account by GitHub user ID (via github_identity table).
    #[tracing::instrument(name = "Postgres::get_account_by_github_id")]
    pub async fn get_account_by_github_id(&self, github_user_id: i64) -> Result<Option<Account>> {
        let row = sqlx::query!(
            r#"
            SELECT a.id, a.email, a.name, a.disabled_at, a.created_at
            FROM account a
            JOIN github_identity gi ON a.id = gi.account_id
            WHERE gi.github_user_id = $1
            "#,
            github_user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch account by github id")?;

        Ok(row.map(|r| Account {
            id: AccountId::from_i64(r.id),
            email: r.email,
            name: r.name,
            disabled_at: r.disabled_at,
            created_at: r.created_at,
        }))
    }

    /// Update an account's email address.
    #[tracing::instrument(name = "Postgres::update_account_email")]
    pub async fn update_account_email(&self, account_id: AccountId, email: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET email = $2
            WHERE id = $1
            "#,
            account_id.as_i64(),
            email,
        )
        .execute(&self.pool)
        .await
        .context("update account email")?;

        Ok(())
    }

    /// Update an account's name.
    #[tracing::instrument(name = "Postgres::update_account_name")]
    pub async fn update_account_name(
        &self,
        account_id: AccountId,
        name: Option<&str>,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET name = $2
            WHERE id = $1
            "#,
            account_id.as_i64(),
            name,
        )
        .execute(&self.pool)
        .await
        .context("update account name")?;

        Ok(())
    }

    /// Disable an account, preventing all API access.
    #[tracing::instrument(name = "Postgres::disable_account")]
    pub async fn disable_account(&self, account_id: AccountId) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET disabled_at = NOW()
            WHERE id = $1
            "#,
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("disable account")?;

        Ok(())
    }

    /// Re-enable a previously disabled account.
    #[tracing::instrument(name = "Postgres::enable_account")]
    pub async fn enable_account(&self, account_id: AccountId) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE account
            SET disabled_at = NULL
            WHERE id = $1
            "#,
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("enable account")?;

        Ok(())
    }
}

// =============================================================================
// GitHub Identity Operations
// =============================================================================

/// A GitHub identity record from the database.
#[derive(Clone, Debug)]
pub struct GitHubIdentity {
    pub id: i64,
    pub account_id: AccountId,
    pub github_user_id: i64,
    pub github_username: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Postgres {
    /// Link a GitHub identity to an account.
    #[tracing::instrument(name = "Postgres::link_github_identity")]
    pub async fn link_github_identity(
        &self,
        account_id: AccountId,
        github_user_id: i64,
        github_username: &str,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO github_identity (account_id, github_user_id, github_username)
            VALUES ($1, $2, $3)
            "#,
            account_id.as_i64(),
            github_user_id,
            github_username,
        )
        .execute(&self.pool)
        .await
        .context("link github identity")?;

        Ok(())
    }

    /// Get the GitHub identity for an account.
    #[tracing::instrument(name = "Postgres::get_github_identity")]
    pub async fn get_github_identity(
        &self,
        account_id: AccountId,
    ) -> Result<Option<GitHubIdentity>> {
        let row = sqlx::query!(
            r#"
            SELECT id, account_id, github_user_id, github_username, created_at, updated_at
            FROM github_identity
            WHERE account_id = $1
            "#,
            account_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch github identity")?;

        Ok(row.map(|r| GitHubIdentity {
            id: r.id,
            account_id: AccountId::from_i64(r.account_id),
            github_user_id: r.github_user_id,
            github_username: r.github_username,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    /// Update the GitHub username for an identity.
    #[tracing::instrument(name = "Postgres::update_github_username")]
    pub async fn update_github_username(
        &self,
        account_id: AccountId,
        github_username: &str,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE github_identity
            SET github_username = $2, updated_at = NOW()
            WHERE account_id = $1
            "#,
            account_id.as_i64(),
            github_username,
        )
        .execute(&self.pool)
        .await
        .context("update github username")?;

        Ok(())
    }
}

// =============================================================================
// Session Operations
// =============================================================================

/// A user session record from the database.
#[derive(Clone, Debug)]
pub struct UserSession {
    pub id: SessionId,
    pub account_id: AccountId,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub last_accessed_at: OffsetDateTime,
}

impl Postgres {
    /// Create a new user session.
    ///
    /// The session token should be generated using
    /// `crypto::generate_session_token()`. The token is hashed before
    /// storage.
    #[tracing::instrument(name = "Postgres::create_session", skip(token))]
    pub async fn create_session(
        &self,
        account_id: AccountId,
        token: &SessionToken,
        expires_at: OffsetDateTime,
    ) -> Result<SessionId> {
        let hash = TokenHash::new(token.expose());
        let row = sqlx::query!(
            r#"
            INSERT INTO user_session (account_id, session_token, expires_at)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
            account_id.as_i64(),
            hash.as_bytes(),
            expires_at,
        )
        .fetch_one(&self.pool)
        .await
        .context("create session")?;

        Ok(SessionId::from_i64(row.id))
    }

    /// Validate a session token and return the session context.
    ///
    /// Returns `None` if the token is invalid, expired, or the account is
    /// disabled. On successful validation, extends the session expiration
    /// (sliding window) and updates `last_accessed_at`.
    #[tracing::instrument(name = "Postgres::validate_session", skip(token))]
    pub async fn validate_session(&self, token: &SessionToken) -> Result<Option<SessionContext>> {
        let hash = TokenHash::new(token.expose());
        let row = sqlx::query!(
            r#"
            SELECT us.id, us.account_id
            FROM user_session us
            JOIN account a ON us.account_id = a.id
            WHERE us.session_token = $1
              AND us.expires_at > NOW()
              AND a.disabled_at IS NULL
            "#,
            hash.as_bytes(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("validate session")?;

        let Some(row) = row else {
            return Ok(None);
        };

        // Update last_accessed_at and extend expiration (sliding window: 24 hours from
        // now)
        sqlx::query!(
            r#"
            UPDATE user_session
            SET last_accessed_at = NOW(),
                expires_at = NOW() + INTERVAL '24 hours'
            WHERE id = $1
            "#,
            row.id,
        )
        .execute(&self.pool)
        .await
        .context("update session last_accessed_at and expires_at")?;

        Ok(Some(SessionContext {
            account_id: AccountId::from_i64(row.account_id),
            session_token: token.clone(),
        }))
    }

    /// Revoke a specific session.
    #[tracing::instrument(name = "Postgres::revoke_session", skip(token))]
    pub async fn revoke_session(&self, token: &SessionToken) -> Result<bool> {
        let hash = TokenHash::new(token.expose());
        let result = sqlx::query!(
            r#"
            DELETE FROM user_session
            WHERE session_token = $1
            "#,
            hash.as_bytes(),
        )
        .execute(&self.pool)
        .await
        .context("revoke session")?;

        Ok(result.rows_affected() > 0)
    }

    /// Revoke all sessions for an account.
    #[tracing::instrument(name = "Postgres::revoke_all_sessions")]
    pub async fn revoke_all_sessions(&self, account_id: AccountId) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM user_session
            WHERE account_id = $1
            "#,
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("revoke all sessions")?;

        Ok(result.rows_affected())
    }

    /// Extend a session's expiration time.
    #[tracing::instrument(name = "Postgres::extend_session", skip(token))]
    pub async fn extend_session(
        &self,
        token: &SessionToken,
        new_expires_at: OffsetDateTime,
    ) -> Result<bool> {
        let hash = TokenHash::new(token.expose());
        let result = sqlx::query!(
            r#"
            UPDATE user_session
            SET expires_at = $2, last_accessed_at = NOW()
            WHERE session_token = $1
            "#,
            hash.as_bytes(),
            new_expires_at,
        )
        .execute(&self.pool)
        .await
        .context("extend session")?;

        Ok(result.rows_affected() > 0)
    }

    /// Clean up expired sessions.
    ///
    /// Returns the number of sessions deleted.
    #[tracing::instrument(name = "Postgres::cleanup_expired_sessions")]
    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM user_session
            WHERE expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .context("cleanup expired sessions")?;

        Ok(result.rows_affected())
    }
}

// =============================================================================
// OAuth State Operations
// =============================================================================

/// An OAuth state record from the database.
#[derive(Clone, Debug)]
pub struct OAuthState {
    pub id: i64,
    pub state_token: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

impl Postgres {
    /// Store OAuth state for the authorization flow.
    #[tracing::instrument(name = "Postgres::store_oauth_state", skip(pkce_verifier))]
    pub async fn store_oauth_state(
        &self,
        state_token: &str,
        pkce_verifier: &str,
        redirect_uri: &str,
        expires_at: OffsetDateTime,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO oauth_state (state_token, pkce_verifier, redirect_uri, expires_at)
            VALUES ($1, $2, $3, $4)
            "#,
            state_token,
            pkce_verifier,
            redirect_uri,
            expires_at,
        )
        .execute(&self.pool)
        .await
        .context("store oauth state")?;

        Ok(())
    }

    /// Consume OAuth state (fetch and delete atomically).
    ///
    /// Returns `None` if the state doesn't exist or has expired.
    #[tracing::instrument(name = "Postgres::consume_oauth_state")]
    pub async fn consume_oauth_state(&self, state_token: &str) -> Result<Option<OAuthState>> {
        let row = sqlx::query!(
            r#"
            DELETE FROM oauth_state
            WHERE state_token = $1 AND expires_at > NOW()
            RETURNING id, state_token, pkce_verifier, redirect_uri, created_at, expires_at
            "#,
            state_token,
        )
        .fetch_optional(&self.pool)
        .await
        .context("consume oauth state")?;

        Ok(row.map(|r| OAuthState {
            id: r.id,
            state_token: r.state_token,
            pkce_verifier: r.pkce_verifier,
            redirect_uri: r.redirect_uri,
            created_at: r.created_at,
            expires_at: r.expires_at,
        }))
    }

    /// Clean up expired OAuth state records.
    ///
    /// Returns the number of records deleted.
    #[tracing::instrument(name = "Postgres::cleanup_expired_oauth_state")]
    pub async fn cleanup_expired_oauth_state(&self) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM oauth_state
            WHERE expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .context("cleanup expired oauth state")?;

        Ok(result.rows_affected())
    }
}

// =============================================================================
// Organization Operations
// =============================================================================

/// An organization record from the database.
#[derive(Clone, Debug)]
pub struct Organization {
    pub id: OrgId,
    pub name: String,
    pub created_at: OffsetDateTime,
}

/// An organization with the user's role in it.
#[derive(Clone, Debug)]
pub struct OrganizationWithRole {
    pub organization: Organization,
    pub role: OrgRole,
}

impl Postgres {
    /// Create a new organization.
    #[tracing::instrument(name = "Postgres::create_organization")]
    pub async fn create_organization(&self, name: &str) -> Result<OrgId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO organization (name)
            VALUES ($1)
            RETURNING id
            "#,
            name,
        )
        .fetch_one(&self.pool)
        .await
        .context("create organization")?;

        Ok(OrgId::from_i64(row.id))
    }

    /// Get an organization by ID.
    #[tracing::instrument(name = "Postgres::get_organization")]
    pub async fn get_organization(&self, org_id: OrgId) -> Result<Option<Organization>> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, created_at
            FROM organization
            WHERE id = $1
            "#,
            org_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("fetch organization")?;

        Ok(row.map(|r| Organization {
            id: OrgId::from_i64(r.id),
            name: r.name,
            created_at: r.created_at,
        }))
    }

    /// List all organizations an account is a member of.
    #[tracing::instrument(name = "Postgres::list_organizations_for_account")]
    pub async fn list_organizations_for_account(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<OrganizationWithRole>> {
        let rows = sqlx::query!(
            r#"
            SELECT o.id, o.name, o.created_at, r.name as role_name
            FROM organization o
            JOIN organization_member om ON o.id = om.organization_id
            JOIN organization_role r ON om.role_id = r.id
            WHERE om.account_id = $1
            ORDER BY o.name
            "#,
            account_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list organizations for account")?;

        rows.into_iter()
            .map(|r| {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(OrganizationWithRole {
                    organization: Organization {
                        id: OrgId::from_i64(r.id),
                        name: r.name,
                        created_at: r.created_at,
                    },
                    role,
                })
            })
            .collect()
    }
}

// =============================================================================
// Membership Operations
// =============================================================================

/// An organization member record from the database.
#[derive(Clone, Debug)]
pub struct OrganizationMember {
    pub account_id: AccountId,
    pub email: String,
    pub name: Option<String>,
    pub role: OrgRole,
    pub created_at: OffsetDateTime,
}

impl Postgres {
    /// Add a member to an organization.
    #[tracing::instrument(name = "Postgres::add_organization_member")]
    pub async fn add_organization_member(
        &self,
        org_id: OrgId,
        account_id: AccountId,
        role: OrgRole,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = $3))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
            role.as_db_name(),
        )
        .execute(&self.pool)
        .await
        .context("add organization member")?;

        Ok(())
    }

    /// Remove a member from an organization.
    #[tracing::instrument(name = "Postgres::remove_organization_member")]
    pub async fn remove_organization_member(
        &self,
        org_id: OrgId,
        account_id: AccountId,
    ) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            DELETE FROM organization_member
            WHERE organization_id = $1 AND account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("remove organization member")?;

        Ok(result.rows_affected() > 0)
    }

    /// Update a member's role in an organization.
    #[tracing::instrument(name = "Postgres::update_member_role")]
    pub async fn update_member_role(
        &self,
        org_id: OrgId,
        account_id: AccountId,
        role: OrgRole,
    ) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE organization_member
            SET role_id = (SELECT id FROM organization_role WHERE name = $3)
            WHERE organization_id = $1 AND account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
            role.as_db_name(),
        )
        .execute(&self.pool)
        .await
        .context("update member role")?;

        Ok(result.rows_affected() > 0)
    }

    /// Get a member's role in an organization.
    #[tracing::instrument(name = "Postgres::get_member_role")]
    pub async fn get_member_role(
        &self,
        org_id: OrgId,
        account_id: AccountId,
    ) -> Result<Option<OrgRole>> {
        let row = sqlx::query!(
            r#"
            SELECT r.name as role_name
            FROM organization_member om
            JOIN organization_role r ON om.role_id = r.id
            WHERE om.organization_id = $1 AND om.account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("get member role")?;

        match row {
            Some(r) => {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(Some(role))
            }
            None => Ok(None),
        }
    }

    /// List all members of an organization.
    #[tracing::instrument(name = "Postgres::list_organization_members")]
    pub async fn list_organization_members(
        &self,
        org_id: OrgId,
    ) -> Result<Vec<OrganizationMember>> {
        let rows = sqlx::query!(
            r#"
            SELECT a.id as account_id, a.email, a.name, r.name as role_name, om.created_at
            FROM organization_member om
            JOIN account a ON om.account_id = a.id
            JOIN organization_role r ON om.role_id = r.id
            WHERE om.organization_id = $1
            ORDER BY a.email
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list organization members")?;

        rows.into_iter()
            .map(|r| {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(OrganizationMember {
                    account_id: AccountId::from_i64(r.account_id),
                    email: r.email,
                    name: r.name,
                    role,
                    created_at: r.created_at,
                })
            })
            .collect()
    }

    /// Check if an account is the last admin of an organization.
    #[tracing::instrument(name = "Postgres::is_last_admin")]
    pub async fn is_last_admin(&self, org_id: OrgId, account_id: AccountId) -> Result<bool> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM organization_member om
            JOIN organization_role r ON om.role_id = r.id
            WHERE om.organization_id = $1 AND r.name = 'admin'
            "#,
            org_id.as_i64(),
        )
        .fetch_one(&self.pool)
        .await
        .context("count admins")?;

        let admin_count = row.count.unwrap_or(0);
        if admin_count != 1 {
            return Ok(false);
        }

        // Check if the given account is that one admin
        let is_admin = self
            .get_member_role(org_id, account_id)
            .await?
            .is_some_and(|role| role.is_admin());

        Ok(is_admin)
    }
}

// =============================================================================
// Invitation Operations
// =============================================================================

/// An invitation record from the database.
#[derive(Clone, Debug)]
pub struct Invitation {
    pub id: InvitationId,
    pub organization_id: OrgId,
    pub role: OrgRole,
    pub created_by: AccountId,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub max_uses: Option<i32>,
    pub use_count: i32,
    pub revoked_at: Option<OffsetDateTime>,
}

/// Public invitation info (for preview without authentication).
#[derive(Clone, Debug)]
pub struct InvitationPreview {
    pub organization_name: String,
    pub role: OrgRole,
    pub expires_at: OffsetDateTime,
    pub valid: bool,
}

impl Postgres {
    /// Create a new invitation.
    ///
    /// The token should be generated using
    /// `crypto::generate_invitation_token()`.
    #[tracing::instrument(name = "Postgres::create_invitation", skip(token))]
    pub async fn create_invitation(
        &self,
        org_id: OrgId,
        token: &str,
        role: OrgRole,
        created_by: AccountId,
        expires_at: OffsetDateTime,
        max_uses: Option<i32>,
    ) -> Result<InvitationId> {
        let row = sqlx::query!(
            r#"
            INSERT INTO organization_invitation
                (organization_id, token, role_id, created_by, expires_at, max_uses)
            VALUES
                ($1, $2, (SELECT id FROM organization_role WHERE name = $3), $4, $5, $6)
            RETURNING id
            "#,
            org_id.as_i64(),
            token,
            role.as_db_name(),
            created_by.as_i64(),
            expires_at,
            max_uses,
        )
        .fetch_one(&self.pool)
        .await
        .context("create invitation")?;

        Ok(InvitationId::from_i64(row.id))
    }

    /// Get an invitation by its token.
    #[tracing::instrument(name = "Postgres::get_invitation_by_token", skip(token))]
    pub async fn get_invitation_by_token(&self, token: &str) -> Result<Option<Invitation>> {
        let row = sqlx::query!(
            r#"
            SELECT i.id, i.organization_id, r.name as role_name, i.created_by,
                   i.created_at, i.expires_at, i.max_uses, i.use_count, i.revoked_at
            FROM organization_invitation i
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.token = $1
            "#,
            token,
        )
        .fetch_optional(&self.pool)
        .await
        .context("get invitation by token")?;

        match row {
            Some(r) => {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(Some(Invitation {
                    id: InvitationId::from_i64(r.id),
                    organization_id: OrgId::from_i64(r.organization_id),
                    role,
                    created_by: AccountId::from_i64(r.created_by),
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                    max_uses: r.max_uses,
                    use_count: r.use_count,
                    revoked_at: r.revoked_at,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get public invitation info for preview (without authentication).
    #[tracing::instrument(name = "Postgres::get_invitation_preview", skip(token))]
    pub async fn get_invitation_preview(&self, token: &str) -> Result<Option<InvitationPreview>> {
        let row = sqlx::query!(
            r#"
            SELECT o.name as org_name, r.name as role_name, i.expires_at, i.revoked_at,
                   i.max_uses, i.use_count
            FROM organization_invitation i
            JOIN organization o ON i.organization_id = o.id
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.token = $1
            "#,
            token,
        )
        .fetch_optional(&self.pool)
        .await
        .context("get invitation preview")?;

        match row {
            Some(r) => {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                let now = OffsetDateTime::now_utc();
                let valid = r.revoked_at.is_none()
                    && r.expires_at > now
                    && r.max_uses.is_none_or(|max| r.use_count < max);
                Ok(Some(InvitationPreview {
                    organization_name: r.org_name,
                    role,
                    expires_at: r.expires_at,
                    valid,
                }))
            }
            None => Ok(None),
        }
    }

    /// Accept an invitation (atomic: increment use_count, add member, log
    /// redemption).
    ///
    /// Returns the organization info if successful.
    #[tracing::instrument(name = "Postgres::accept_invitation", skip(token))]
    pub async fn accept_invitation(
        &self,
        token: &str,
        account_id: AccountId,
    ) -> Result<AcceptInvitationResult> {
        let mut tx = self.pool.begin().await?;

        // Get and lock the invitation
        let invitation = sqlx::query!(
            r#"
            SELECT i.id, i.organization_id, r.name as role_name, i.expires_at,
                   i.max_uses, i.use_count, i.revoked_at
            FROM organization_invitation i
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.token = $1
            FOR UPDATE
            "#,
            token,
        )
        .fetch_optional(tx.as_mut())
        .await
        .context("fetch invitation for update")?;

        let Some(inv) = invitation else {
            return Ok(AcceptInvitationResult::NotFound);
        };

        // Check if expired, revoked, or at max uses
        let now = OffsetDateTime::now_utc();
        if inv.revoked_at.is_some() {
            return Ok(AcceptInvitationResult::Revoked);
        }
        if inv.expires_at <= now {
            return Ok(AcceptInvitationResult::Expired);
        }
        if inv.max_uses.is_some_and(|max| inv.use_count >= max) {
            return Ok(AcceptInvitationResult::MaxUsesReached);
        }

        let org_id = OrgId::from_i64(inv.organization_id);
        let role = OrgRole::from_db_name(&inv.role_name)
            .ok_or_else(|| eyre!("unknown role: {}", inv.role_name))?;

        // Check if already a member
        let existing = sqlx::query!(
            r#"
            SELECT 1 as exists
            FROM organization_member
            WHERE organization_id = $1 AND account_id = $2
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .fetch_optional(tx.as_mut())
        .await
        .context("check existing membership")?;

        if existing.is_some() {
            return Ok(AcceptInvitationResult::AlreadyMember);
        }

        // Increment use count
        sqlx::query!(
            r#"
            UPDATE organization_invitation
            SET use_count = use_count + 1
            WHERE id = $1
            "#,
            inv.id,
        )
        .execute(tx.as_mut())
        .await
        .context("increment use count")?;

        // Add member
        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = $3))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
            role.as_db_name(),
        )
        .execute(tx.as_mut())
        .await
        .context("add organization member")?;

        // Log redemption
        sqlx::query!(
            r#"
            INSERT INTO invitation_redemption (invitation_id, account_id)
            VALUES ($1, $2)
            "#,
            inv.id,
            account_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("log invitation redemption")?;

        // Get organization info
        let org = sqlx::query!(
            r#"
            SELECT name FROM organization WHERE id = $1
            "#,
            org_id.as_i64(),
        )
        .fetch_one(tx.as_mut())
        .await
        .context("fetch organization name")?;

        tx.commit().await?;

        Ok(AcceptInvitationResult::Success {
            organization_id: org_id,
            organization_name: org.name,
            role,
        })
    }

    /// Revoke an invitation.
    #[tracing::instrument(name = "Postgres::revoke_invitation")]
    pub async fn revoke_invitation(&self, invitation_id: InvitationId) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE organization_invitation
            SET revoked_at = NOW()
            WHERE id = $1 AND revoked_at IS NULL
            "#,
            invitation_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("revoke invitation")?;

        Ok(result.rows_affected() > 0)
    }

    /// List all invitations for an organization.
    #[tracing::instrument(name = "Postgres::list_invitations")]
    pub async fn list_invitations(&self, org_id: OrgId) -> Result<Vec<Invitation>> {
        let rows = sqlx::query!(
            r#"
            SELECT i.id, i.organization_id, r.name as role_name, i.created_by,
                   i.created_at, i.expires_at, i.max_uses, i.use_count, i.revoked_at
            FROM organization_invitation i
            JOIN organization_role r ON i.role_id = r.id
            WHERE i.organization_id = $1
            ORDER BY i.created_at DESC
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list invitations")?;

        rows.into_iter()
            .map(|r| {
                let role = OrgRole::from_db_name(&r.role_name)
                    .ok_or_else(|| eyre!("unknown role: {}", r.role_name))?;
                Ok(Invitation {
                    id: InvitationId::from_i64(r.id),
                    organization_id: OrgId::from_i64(r.organization_id),
                    role,
                    created_by: AccountId::from_i64(r.created_by),
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                    max_uses: r.max_uses,
                    use_count: r.use_count,
                    revoked_at: r.revoked_at,
                })
            })
            .collect()
    }
}

/// Result of accepting an invitation.
#[derive(Clone, Debug)]
pub enum AcceptInvitationResult {
    /// Successfully joined the organization.
    Success {
        organization_id: OrgId,
        organization_name: String,
        role: OrgRole,
    },
    /// Invitation not found.
    NotFound,
    /// Invitation has been revoked.
    Revoked,
    /// Invitation has expired.
    Expired,
    /// Invitation has reached its maximum uses.
    MaxUsesReached,
    /// Account is already a member of the organization.
    AlreadyMember,
}

// =============================================================================
// Audit Log Operations
// =============================================================================

impl Postgres {
    /// Log an audit event.
    #[tracing::instrument(name = "Postgres::log_audit_event", skip(details))]
    pub async fn log_audit_event(
        &self,
        account_id: Option<AccountId>,
        organization_id: Option<OrgId>,
        action: &str,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO audit_log (account_id, organization_id, action, details)
            VALUES ($1, $2, $3, $4)
            "#,
            account_id.map(|id| id.as_i64()),
            organization_id.map(|id| id.as_i64()),
            action,
            details,
        )
        .execute(&self.pool)
        .await
        .context("log audit event")?;

        Ok(())
    }
}

// =============================================================================
// API Key Operations (extended for org_id support)
// =============================================================================

/// An API key record from the database.
#[derive(Clone, Debug)]
pub struct ApiKey {
    pub id: ApiKeyId,
    pub account_id: AccountId,
    pub organization_id: Option<OrgId>,
    pub name: String,
    pub created_at: OffsetDateTime,
    pub accessed_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

impl Postgres {
    /// Create a new API key with optional organization scope.
    ///
    /// Returns the raw token (only time it's available in plaintext).
    #[tracing::instrument(name = "Postgres::create_api_key")]
    pub async fn create_api_key(
        &self,
        account_id: AccountId,
        name: &str,
        organization_id: Option<OrgId>,
    ) -> Result<(ApiKeyId, RawToken)> {
        let token = crate::crypto::generate_api_key();
        let hash = TokenHash::new(token.expose());

        let row = sqlx::query!(
            r#"
            INSERT INTO api_key (account_id, name, hash, organization_id)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
            account_id.as_i64(),
            name,
            hash.as_bytes(),
            organization_id.map(|id| id.as_i64()),
        )
        .fetch_one(&self.pool)
        .await
        .context("create api key")?;

        Ok((ApiKeyId::from_i64(row.id), token))
    }

    /// List personal API keys (no organization scope) for an account.
    #[tracing::instrument(name = "Postgres::list_personal_api_keys")]
    pub async fn list_personal_api_keys(&self, account_id: AccountId) -> Result<Vec<ApiKey>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, account_id, organization_id, name, created_at, accessed_at, revoked_at
            FROM api_key
            WHERE account_id = $1 AND organization_id IS NULL AND revoked_at IS NULL
            ORDER BY created_at DESC
            "#,
            account_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list personal api keys")?;

        Ok(rows
            .into_iter()
            .map(|r| ApiKey {
                id: ApiKeyId::from_i64(r.id),
                account_id: AccountId::from_i64(r.account_id),
                organization_id: r.organization_id.map(OrgId::from_i64),
                name: r.name,
                created_at: r.created_at,
                accessed_at: r.accessed_at,
                revoked_at: r.revoked_at,
            })
            .collect())
    }

    /// List organization-scoped API keys for an account in a specific org.
    #[tracing::instrument(name = "Postgres::list_org_api_keys")]
    pub async fn list_org_api_keys(
        &self,
        account_id: AccountId,
        org_id: OrgId,
    ) -> Result<Vec<ApiKey>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, account_id, organization_id, name, created_at, accessed_at, revoked_at
            FROM api_key
            WHERE account_id = $1 AND organization_id = $2 AND revoked_at IS NULL
            ORDER BY created_at DESC
            "#,
            account_id.as_i64(),
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list org api keys")?;

        Ok(rows
            .into_iter()
            .map(|r| ApiKey {
                id: ApiKeyId::from_i64(r.id),
                account_id: AccountId::from_i64(r.account_id),
                organization_id: r.organization_id.map(OrgId::from_i64),
                name: r.name,
                created_at: r.created_at,
                accessed_at: r.accessed_at,
                revoked_at: r.revoked_at,
            })
            .collect())
    }

    /// Revoke an API key by ID.
    #[tracing::instrument(name = "Postgres::revoke_api_key")]
    pub async fn revoke_api_key(&self, key_id: ApiKeyId) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE api_key
            SET revoked_at = NOW()
            WHERE id = $1 AND revoked_at IS NULL
            "#,
            key_id.as_i64(),
        )
        .execute(&self.pool)
        .await
        .context("revoke api key")?;

        Ok(result.rows_affected() > 0)
    }

    /// Get an API key by ID (for authorization checks).
    #[tracing::instrument(name = "Postgres::get_api_key")]
    pub async fn get_api_key(&self, key_id: ApiKeyId) -> Result<Option<ApiKey>> {
        let row = sqlx::query!(
            r#"
            SELECT id, account_id, organization_id, name, created_at, accessed_at, revoked_at
            FROM api_key
            WHERE id = $1
            "#,
            key_id.as_i64(),
        )
        .fetch_optional(&self.pool)
        .await
        .context("get api key")?;

        Ok(row.map(|r| ApiKey {
            id: ApiKeyId::from_i64(r.id),
            account_id: AccountId::from_i64(r.account_id),
            organization_id: r.organization_id.map(OrgId::from_i64),
            name: r.name,
            created_at: r.created_at,
            accessed_at: r.accessed_at,
            revoked_at: r.revoked_at,
        }))
    }

    /// List all API keys for an organization (from all members).
    ///
    /// Includes account email for display purposes.
    #[tracing::instrument(name = "Postgres::list_all_org_api_keys")]
    pub async fn list_all_org_api_keys(&self, org_id: OrgId) -> Result<Vec<OrgApiKey>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                api_key.id,
                api_key.account_id,
                api_key.name,
                api_key.created_at,
                api_key.accessed_at,
                account.email as account_email
            FROM api_key
            JOIN account ON api_key.account_id = account.id
            WHERE api_key.organization_id = $1 AND api_key.revoked_at IS NULL
            ORDER BY api_key.created_at DESC
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list all org api keys")?;

        Ok(rows
            .into_iter()
            .map(|r| OrgApiKey {
                id: ApiKeyId::from_i64(r.id),
                account_id: AccountId::from_i64(r.account_id),
                name: r.name,
                account_email: r.account_email,
                created_at: r.created_at,
                accessed_at: r.accessed_at,
            })
            .collect())
    }
}

/// An API key with account email (for org listing).
#[derive(Debug)]
pub struct OrgApiKey {
    pub id: ApiKeyId,
    pub account_id: AccountId,
    pub name: String,
    pub account_email: String,
    pub created_at: OffsetDateTime,
    pub accessed_at: OffsetDateTime,
}

// =============================================================================
// Bot Account Operations
// =============================================================================

/// A bot account record from the database.
///
/// Bot accounts are organization-scoped accounts without GitHub identity,
/// used for CI systems and automation.
#[derive(Clone, Debug)]
pub struct BotAccount {
    pub id: AccountId,
    pub name: Option<String>,
    pub email: String,
    pub created_at: OffsetDateTime,
}

impl Postgres {
    /// Create a bot account for an organization.
    ///
    /// Bot accounts:
    /// - Have no GitHub identity
    /// - Belong to exactly one organization (as member role by default)
    /// - Use `email` field for the responsible person's contact email
    /// - Get an initial API key created
    ///
    /// Returns the account ID and the API key token.
    #[tracing::instrument(name = "Postgres::create_bot_account")]
    pub async fn create_bot_account(
        &self,
        org_id: OrgId,
        name: &str,
        responsible_email: &str,
    ) -> Result<(AccountId, crate::auth::RawToken)> {
        let mut tx = self.pool.begin().await?;

        // Create the account
        let row = sqlx::query!(
            r#"
            INSERT INTO account (email, name)
            VALUES ($1, $2)
            RETURNING id
            "#,
            responsible_email,
            name,
        )
        .fetch_one(tx.as_mut())
        .await
        .context("create bot account")?;

        let account_id = AccountId::from_i64(row.id);

        // Add as member of the organization
        sqlx::query!(
            r#"
            INSERT INTO organization_member (organization_id, account_id, role_id)
            VALUES ($1, $2, (SELECT id FROM organization_role WHERE name = 'member'))
            "#,
            org_id.as_i64(),
            account_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("add bot to organization")?;

        // Create an initial API key for the bot
        let token = crate::crypto::generate_api_key();
        let hash = TokenHash::new(token.expose());
        let key_name = format!("{} API Key", name);

        sqlx::query!(
            r#"
            INSERT INTO api_key (account_id, name, hash, organization_id)
            VALUES ($1, $2, $3, $4)
            "#,
            account_id.as_i64(),
            key_name,
            hash.as_bytes(),
            org_id.as_i64(),
        )
        .execute(tx.as_mut())
        .await
        .context("create bot api key")?;

        tx.commit().await?;

        Ok((account_id, token))
    }

    /// List bot accounts for an organization.
    ///
    /// Bot accounts are accounts that:
    /// - Are members of the organization
    /// - Have no GitHub identity linked
    #[tracing::instrument(name = "Postgres::list_bot_accounts")]
    pub async fn list_bot_accounts(&self, org_id: OrgId) -> Result<Vec<BotAccount>> {
        let rows = sqlx::query!(
            r#"
            SELECT a.id, a.name, a.email, a.created_at
            FROM account a
            JOIN organization_member om ON a.id = om.account_id
            WHERE om.organization_id = $1
              AND NOT EXISTS (
                  SELECT 1 FROM github_identity gi WHERE gi.account_id = a.id
              )
            ORDER BY a.created_at DESC
            "#,
            org_id.as_i64(),
        )
        .fetch_all(&self.pool)
        .await
        .context("list bot accounts")?;

        Ok(rows
            .into_iter()
            .map(|r| BotAccount {
                id: AccountId::from_i64(r.id),
                name: r.name,
                email: r.email,
                created_at: r.created_at,
            })
            .collect())
    }
}
