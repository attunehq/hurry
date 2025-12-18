//! Cargo cache database operations.

use std::collections::{HashMap, HashSet};

use clients::courier::v1::{
    GlibcVersion, Key, SavedUnit, SavedUnitHash,
    cache::{CargoRestoreRequest, CargoSaveRequest},
};
use color_eyre::{Result, eyre::Context};
use futures::StreamExt;
use tracing::{debug, trace};

use super::Postgres;
use crate::auth::AuthenticatedToken;

impl Postgres {
    #[tracing::instrument(name = "Postgres::save_cargo_cache", skip(auth))]
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

    #[tracing::instrument(name = "Postgres::cargo_cache_restore", skip(auth))]
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

            let unit_hash = &row.unit_hash;
            let key = unit_hash.clone().into();
            let unit = serde_json::from_value::<SavedUnit>(row.data)
                .with_context(|| format!("deserialize value for cache key: {key}"))?;

            // Check for glibc version compatibility for units that compile
            // against glibc.
            trace!(
                %unit_hash,
                host_glibc = ?request.host_glibc_version,
                saved_glibc = ?row.linux_glibc_version,
                "checking glibc compatibility"
            );
            if let Some(ref host_glibc) = request.host_glibc_version {
                let Some(saved_glibc_string) = row.linux_glibc_version else {
                    // Skip units without glibc version info. Note that this
                    // should never happen, since all units with a matching unit
                    // hash will all be on the same target, and all units of a
                    // target either do or do not have glibc version info.
                    debug!(
                        %unit_hash,
                        "skipping unit: no saved glibc version"
                    );
                    continue;
                };
                let saved_glibc = saved_glibc_string.as_str().parse::<GlibcVersion>()?;
                if *host_glibc < saved_glibc {
                    // Skip units with incompatible glibc versions.
                    debug!(
                        %unit_hash,
                        %host_glibc,
                        %saved_glibc,
                        "skipping unit: host glibc too old"
                    );
                    continue;
                }
            } else if let Some(_) = row.linux_glibc_version {
                // Skip units that have glibc version info when host doesn't
                // have glibc version info (i.e., non-linux targets). This is
                // another thing that should never happen.
                debug!(
                    %unit_hash,
                    "skipping unit: host has no glibc but saved unit does"
                );
                continue;
            }

            artifacts.insert(key, unit);
        }

        Ok(artifacts)
    }

    /// Grant an organization access to a CAS key.
    ///
    /// This is idempotent: if the organization already has access, this is a
    /// no-op. The operation atomically upserts the CAS key and grants access.
    ///
    /// Returns `true` if access was newly granted, `false` if the org already
    /// had access.
    #[tracing::instrument(name = "Postgres::grant_cas_access", skip(auth))]
    pub async fn grant_cas_access(&self, auth: &AuthenticatedToken, key: &Key) -> Result<bool> {
        let mut tx = self.pool.begin().await?;

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
        .fetch_one(tx.as_mut())
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
            auth.org_id.as_i64(),
            key_id,
        )
        .execute(tx.as_mut())
        .await
        .context("grant org access to cas key")?;

        tx.commit().await?;

        // If rows_affected is 1, we inserted a new row (newly granted access)
        // If rows_affected is 0, the row already existed (org already had access)
        Ok(result.rows_affected() == 1)
    }

    /// Check if an organization has access to a CAS key.
    #[tracing::instrument(name = "Postgres::check_cas_access", skip(auth))]
    pub async fn check_cas_access(&self, auth: &AuthenticatedToken, key: &Key) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM cas_access
                WHERE organization_id = $1
                AND cas_key_id = (SELECT id FROM cas_key WHERE content = $2)
            ) as "exists!"
            "#,
            auth.org_id.as_i64(),
            key.as_bytes(),
        )
        .fetch_one(&self.pool)
        .await
        .context("check cas access")?;

        Ok(result.exists)
    }

    /// Check which keys from a set the organization has access to.
    /// Returns a HashSet of keys that the organization can access.
    #[tracing::instrument(name = "Postgres::check_cas_access_bulk", skip(auth, keys))]
    pub async fn check_cas_access_bulk(
        &self,
        auth: &AuthenticatedToken,
        keys: &[Key],
    ) -> Result<HashSet<Key>> {
        if keys.is_empty() {
            return Ok(HashSet::new());
        }

        let key_bytes = keys
            .iter()
            .map(|k| k.as_bytes().to_vec())
            .collect::<Vec<_>>();

        let rows = sqlx::query!(
            r#"
            SELECT cas_key.content
            FROM cas_key
            JOIN cas_access ON cas_key.id = cas_access.cas_key_id
            WHERE cas_access.organization_id = $1
            AND cas_key.content = ANY($2)
            "#,
            auth.org_id.as_i64(),
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

    #[tracing::instrument(name = "Postgres::cargo_cache_reset", skip(auth))]
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
