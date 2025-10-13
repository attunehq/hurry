//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use derive_more::Debug;
use sqlx::{PgPool, migrate::Migrator};

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
