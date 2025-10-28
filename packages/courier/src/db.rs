//! Database interface.
//!
//! # Serialization/Deserialization
//!
//! Types in this module do not implement `Serialize` or `Deserialize` because
//! they are internal implementation details for Courier. If you want to
//! serialize or deserialize these types, create public-facing types that do so
//! and are able to convert back and forth with the internal types.

use std::collections::HashMap;

use clients::courier::v1::{
    Key,
    cache::{ArtifactFile, CargoRestoreRequest, CargoSaveRequest},
};
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

#[derive(Debug, Clone)]
struct CargoLibraryUnitBuildRow {
    id: i64,
    content_hash: String,
}

impl Postgres {
    #[tracing::instrument(name = "Postgres::save_cargo_cache")]
    pub async fn cargo_cache_save(&self, request: CargoSaveRequest) -> Result<()> {
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
            let object_key = artifact.object_key.to_hex();
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
                object_key
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
        request: CargoRestoreRequest,
    ) -> Result<Vec<ArtifactFile>, Report> {
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

        let mut artifacts = Vec::<ArtifactFile>::new();
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
            let object_key = Key::from_hex(&row.key).context("parse object key from database")?;
            artifacts.push(
                ArtifactFile::builder()
                    .object_key(object_key)
                    .path(row.path)
                    .mtime_nanos(row.mtime.to_u128().unwrap_or_default())
                    .executable(row.executable)
                    .build(),
            );
        }

        debug!(
            library_unit_build_id = unit_to_restore.id,
            "cache.restore.hit"
        );

        Ok(artifacts)
    }

    /// Restore multiple cargo cache entries in bulk using a single query.
    ///
    /// This is significantly faster than calling `cargo_cache_restore` in a
    /// loop because it issues a single database query instead of N queries.
    ///
    /// The result maps resulting artifact files by the index of the request
    /// that caused them, exactly as if the caller had invoked
    /// [`Postgres::cargo_cache_restore`] for each item in the index.
    #[tracing::instrument(name = "Postgres::cargo_cache_restore_bulk", skip(requests))]
    pub async fn cargo_cache_restore_bulk(
        &self,
        requests: &[CargoRestoreRequest],
    ) -> Result<HashMap<usize, Vec<ArtifactFile>>, Report> {
        if requests.is_empty() {
            return Ok(HashMap::new());
        }

        let mut tx = self.pool.begin().await?;

        let mut request_indices = Vec::new();
        let mut package_names = Vec::new();
        let mut package_versions = Vec::new();
        let mut targets = Vec::new();
        let mut lib_hashes = Vec::new();
        let mut build_comp_hashes = Vec::new();
        let mut build_exec_hashes = Vec::new();
        for (i, request) in requests.iter().enumerate() {
            request_indices.push(i as i32);
            package_names.push(request.package_name.as_str());
            package_versions.push(request.package_version.as_str());
            targets.push(request.target.as_str());
            lib_hashes.push(request.library_crate_compilation_unit_hash.as_str());
            build_comp_hashes.push(request.build_script_compilation_unit_hash.as_deref());
            build_exec_hashes.push(request.build_script_execution_unit_hash.as_deref());
        }

        // Find all matching builds
        let mut build_rows = sqlx::query!(
            r#"
            WITH request_data AS (
                SELECT
                    unnest($1::integer[]) as request_idx,
                    unnest($2::text[]) as package_name,
                    unnest($3::text[]) as package_version,
                    unnest($4::text[]) as target,
                    unnest($5::text[]) as lib_hash,
                    unnest($6::text[]) as build_comp_hash,
                    unnest($7::text[]) as build_exec_hash
            )
            SELECT
                rd.request_idx,
                clb.id as build_id,
                clb.content_hash
            FROM request_data rd
            JOIN cargo_package cp ON cp.name = rd.package_name AND cp.version = rd.package_version
            JOIN cargo_library_unit_build clb ON
                clb.package_id = cp.id
                AND clb.target = rd.target
                AND clb.library_crate_compilation_unit_hash = rd.lib_hash
                AND COALESCE(clb.build_script_compilation_unit_hash, '') = COALESCE(rd.build_comp_hash, '')
                AND COALESCE(clb.build_script_execution_unit_hash, '') = COALESCE(rd.build_exec_hash, '')
            "#,
            &request_indices,
            &package_names as &[&str],
            &package_versions as &[&str],
            &targets as &[&str],
            &lib_hashes as &[&str],
            &build_comp_hashes as &[Option<&str>],
            &build_exec_hashes as &[Option<&str>]
        )
        .fetch(&mut *tx);

        let mut build_id_to_request_idx = HashMap::new();
        let mut request_idx_to_content_hash = HashMap::new();
        while let Some(row) = build_rows.next().await {
            let row = row.context("read row")?;
            let Some(request_idx) = row.request_idx.map(|idx| idx as usize) else {
                bail!("Missing request index for build row: {row:?}");
            };

            match request_idx_to_content_hash.get(&request_idx) {
                None => {
                    request_idx_to_content_hash.insert(request_idx, row.content_hash.clone());
                    build_id_to_request_idx.insert(row.build_id, request_idx);
                }
                Some(existing_hash) if existing_hash != &row.content_hash => {
                    let request = &requests[request_idx];
                    warn!(
                        existing_content_hash = ?existing_hash,
                        new_content_hash = ?row.content_hash,
                        package_name = %request.package_name,
                        package_version = %request.package_version,
                        "cache.restore.content_hash_mismatch"
                    );
                    // Remove this request_idx from consideration
                    build_id_to_request_idx.retain(|_, idx| *idx != request_idx);
                    request_idx_to_content_hash.remove(&request_idx);
                }
                Some(_) => {
                    // Same content hash, just use first build_id
                }
            }
        }

        if build_id_to_request_idx.is_empty() {
            debug!("cache.restore_bulk.all_misses");
            return Ok(HashMap::new());
        }

        drop(build_rows);
        let build_ids = build_id_to_request_idx.keys().copied().collect::<Vec<_>>();
        let mut artifact_rows = sqlx::query!(
            r#"
            SELECT
                clba.library_unit_build_id as build_id,
                co.key as object_key,
                clba.path,
                clba.mtime,
                clba.executable
            FROM cargo_library_unit_build_artifact clba
            JOIN cargo_object co ON clba.object_id = co.id
            WHERE clba.library_unit_build_id = ANY($1)
            "#,
            &build_ids
        )
        .fetch(&mut *tx);

        let mut results_by_request_idx = HashMap::<usize, Vec<ArtifactFile>>::new();
        while let Some(row) = artifact_rows.next().await {
            let row = row.context("read row")?;
            let Some(&request_idx) = build_id_to_request_idx.get(&row.build_id) else {
                bail!("Missing request index for build row: {row:?}");
            };

            let object_key = Key::from_hex(&row.object_key).context("parse object key")?;
            let artifact = ArtifactFile::builder()
                .object_key(object_key)
                .path(row.path)
                .mtime_nanos(row.mtime.to_u128().unwrap_or_default())
                .executable(row.executable)
                .build();

            results_by_request_idx
                .entry(request_idx)
                .or_default()
                .push(artifact);
        }

        debug!(
            hits = results_by_request_idx.len(),
            misses = requests.len() - results_by_request_idx.len(),
            "cache.restore_bulk.complete"
        );

        Ok(results_by_request_idx)
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
