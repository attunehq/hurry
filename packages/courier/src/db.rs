//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

mod account;
mod api_key;
pub mod audit;
mod bot_account;
mod cargo_cache;
mod github_identity;
mod invitation;
mod member;
mod oauth;
mod organization;
mod session;

use std::collections::HashMap;

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use derive_more::Debug;
use sqlx::migrate::Migrate;
use sqlx::{PgPool, migrate::Migrator};

// Re-export types from submodules.
pub use account::{Account, SignupResult};
pub use api_key::{ApiKey, OrgApiKey};
pub use bot_account::BotAccount;
pub use github_identity::GitHubIdentity;
pub use invitation::{AcceptInvitationResult, Invitation, InvitationPreview};
pub use member::OrganizationMember;
pub use oauth::{ExchangeCodeRedemption, OAuthState, RedeemExchangeCodeError};
pub use organization::{Organization, OrganizationWithRole};
pub use session::UserSession;

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
        let applied_versions = applied
            .iter()
            .map(|m| m.version)
            .collect::<std::collections::HashSet<_>>();

        // Expected migrations are the up-migrations defined in the codebase.
        let expected = Self::MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_up_migration())
            .collect::<Vec<_>>();
        let expected_versions = expected
            .iter()
            .map(|m| m.version)
            .collect::<std::collections::HashSet<_>>();

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
