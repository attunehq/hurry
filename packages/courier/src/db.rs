//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use bon::Builder;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, Report, bail},
};
use derive_more::Debug;
use futures::StreamExt;
use num_traits::ToPrimitive;
use sqlx::{PgPool, migrate::Migrator};
use tracing::{debug, warn};

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

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
#[builder(on(String, into))]
pub struct CargoSaveCacheRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
    pub content_hash: String,

    #[debug("{:?}", self.artifacts.len())]
    #[builder(with = |a: impl IntoIterator<Item = impl Into<CargoArtifact>>| a.into_iter().map(|a| a.into()).collect())]
    pub artifacts: Vec<CargoArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
#[builder(on(String, into))]
pub struct CargoArtifact {
    pub object_key: String,
    pub path: String,
    pub mtime_nanos: u128,
    pub executable: bool,
}

impl From<&CargoArtifact> for CargoArtifact {
    fn from(a: &CargoArtifact) -> Self {
        a.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
#[builder(on(String, into))]
pub struct CargoRestoreCacheRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
}

#[derive(Debug, Clone)]
struct CargoLibraryUnitBuildRow {
    id: i64,
    content_hash: String,
}

impl Postgres {
    #[tracing::instrument(name = "Postgres::save_cargo_cache")]
    pub async fn cargo_cache_save(&self, request: CargoSaveCacheRequest) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let package_id = sqlx::query!(
            r#"
            WITH inserted AS (
                INSERT INTO cargo_package (name, version)
                VALUES ($1, $2)
                ON CONFLICT (name, version) DO NOTHING
                RETURNING id
            )
            SELECT id FROM inserted
            UNION ALL
            SELECT id FROM cargo_package WHERE name = $1 AND version = $2
            LIMIT 1
            "#,
            request.package_name,
            request.package_version
        )
        .fetch_one(&mut *tx)
        .await
        .context("upsert package")?
        .id;

        // Library unit builds are intended to be immutable: we only insert a
        // new one if it doesn't already exist. If it does exist and the content
        // hash is different, this indicates an error in how the cache is being
        // used; we don't want to edit the build to upsert the new data.
        let existing_build = sqlx::query_as!(
            CargoLibraryUnitBuildRow,
            r#"
            SELECT id, content_hash
            FROM cargo_library_unit_build
            WHERE package_id = $1
            AND target = $2
            AND library_crate_compilation_unit_hash = $3
            AND COALESCE(build_script_compilation_unit_hash, '') = COALESCE($4, '')
            AND COALESCE(build_script_execution_unit_hash, '') = COALESCE($5, '')
            "#,
            package_id,
            request.target,
            request.library_crate_compilation_unit_hash,
            request.build_script_compilation_unit_hash,
            request.build_script_execution_unit_hash
        )
        .fetch_optional(&mut *tx)
        .await
        .context("check for existing library unit build")?;

        // If it does exist, and the content hash is the same, there is nothing
        // more to do. If it exists but the content hash is different, something
        // has gone wrong with our cache key and we should abort.
        match existing_build {
            Some(existing) if existing.content_hash == request.content_hash => {
                debug!(
                    library_unit_build_id = existing.id,
                    library_unit_build_content_hash = existing.content_hash,
                    "cache.save.already_exists"
                );
                return tx.commit().await.context("commit transaction");
            }
            Some(existing) => {
                bail!(
                    "content hash mismatch for package {}, version {}, unit hash {}: expected {:?}, actual {:?}",
                    request.package_name,
                    request.package_version,
                    request.library_crate_compilation_unit_hash,
                    existing.content_hash,
                    request.content_hash
                );
            }
            None => {}
        }

        let library_unit_build_id = sqlx::query!(
            r#"
            INSERT INTO cargo_library_unit_build (
                package_id,
                target,
                library_crate_compilation_unit_hash,
                build_script_compilation_unit_hash,
                build_script_execution_unit_hash,
                content_hash
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
            package_id,
            request.target,
            request.library_crate_compilation_unit_hash,
            request.build_script_compilation_unit_hash,
            request.build_script_execution_unit_hash,
            request.content_hash
        )
        .fetch_one(&mut *tx)
        .await
        .context("insert library unit build")?
        .id;

        debug!(library_unit_build_id, "cache.save.inserted");

        // TODO: Bulk insert.
        for artifact in request.artifacts {
            let object_id = sqlx::query!(
                r#"
                WITH inserted AS (
                    INSERT INTO cargo_object (key)
                    VALUES ($1)
                    ON CONFLICT (key) DO NOTHING
                    RETURNING id
                )
                SELECT id FROM inserted
                UNION ALL
                SELECT id FROM cargo_object WHERE key = $1
                LIMIT 1
                "#,
                artifact.object_key
            )
            .fetch_one(&mut *tx)
            .await?
            .id;

            let mtime = bigdecimal::BigDecimal::from(artifact.mtime_nanos);
            sqlx::query!(
                r#"
                INSERT INTO cargo_library_unit_build_artifact (
                    library_unit_build_id,
                    object_id,
                    path,
                    mtime,
                    executable
                ) VALUES ($1, $2, $3, $4, $5)
                "#,
                library_unit_build_id,
                object_id,
                artifact.path,
                mtime,
                artifact.executable
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await.context("commit transaction")
    }

    #[tracing::instrument(name = "Postgres::cargo_cache_restore")]
    pub async fn cargo_cache_restore(
        &self,
        request: CargoRestoreCacheRequest,
    ) -> Result<Vec<CargoArtifact>, Report> {
        let mut tx = self.pool.begin().await?;

        let unit_to_restore = {
            // We would normally use a `split_first` approach here, but this
            // streaming approach allows us to get the same result without
            // buffering the entire collection.
            let mut unit_build = Option::<CargoLibraryUnitBuildRow>::None;
            let mut rows = sqlx::query_as!(
                CargoLibraryUnitBuildRow,
                r#"
                SELECT
                    cargo_library_unit_build.id,
                    cargo_library_unit_build.content_hash
                FROM cargo_package
                JOIN cargo_library_unit_build ON cargo_package.id = cargo_library_unit_build.package_id
                WHERE
                    cargo_package.name = $1
                    AND cargo_package.version = $2
                    AND target = $3
                    AND library_crate_compilation_unit_hash = $4
                    AND COALESCE(build_script_compilation_unit_hash, '') = COALESCE($5, '')
                    AND COALESCE(build_script_execution_unit_hash, '') = COALESCE($6, '')
                "#,
                request.package_name,
                request.package_version,
                request.target,
                request.library_crate_compilation_unit_hash,
                request.build_script_compilation_unit_hash,
                request.build_script_execution_unit_hash
            )
            .fetch(&mut *tx);
            while let Some(row) = rows.next().await {
                let row = row
                    .context("query library unit build")
                    .with_section(|| format!("{request:#?}").header("Request:"))?;
                match unit_build.as_ref() {
                    None => unit_build = Some(row),

                    // Multiple builds with same unit hashes but different
                    // content_hash: the cache key is insufficient to uniquely
                    // identify builds. We log the warning for our
                    // logs/debugging and otherwise present this to the client
                    // as a cache miss.
                    Some(existing) if existing.content_hash != row.content_hash => {
                        warn!(
                            existing_content_hash = ?existing.content_hash,
                            new_content_hash = ?row.content_hash,
                            package_name = %request.package_name,
                            package_version = %request.package_version,
                            "cache.restore.content_hash_mismatch"
                        );
                        return Ok(vec![]);
                    }

                    // Multiple builds with same unit hashes AND same
                    // content_hash are perfectly fine; in that case we could
                    // restore any of them without issue so we just restore the
                    // first one.
                    Some(_) => {}
                }
            }
            match unit_build {
                Some(unit_build) => unit_build,
                None => {
                    debug!("cache.restore.miss");
                    return Ok(vec![]);
                }
            }
        };

        let mut artifacts = Vec::<CargoArtifact>::new();
        let mut rows = sqlx::query!(
            r#"
            SELECT
                cargo_object.key,
                cargo_library_unit_build_artifact.path,
                cargo_library_unit_build_artifact.mtime,
                cargo_library_unit_build_artifact.executable
            FROM cargo_library_unit_build_artifact
            JOIN cargo_object ON cargo_library_unit_build_artifact.object_id = cargo_object.id
            WHERE
                cargo_library_unit_build_artifact.library_unit_build_id = $1
            "#,
            unit_to_restore.id
        )
        .fetch(&mut *tx);
        while let Some(row) = rows.next().await {
            let row = row
                .context("query artifacts")
                .with_section(|| format!("{unit_to_restore:#?}").header("Library unit build:"))?;
            artifacts.push(CargoArtifact {
                object_key: row.key,
                path: row.path,
                mtime_nanos: row.mtime.to_u128().unwrap_or_default(),
                executable: row.executable,
            });
        }

        debug!(
            library_unit_build_id = unit_to_restore.id,
            "cache.restore.hit"
        );

        Ok(artifacts)
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
