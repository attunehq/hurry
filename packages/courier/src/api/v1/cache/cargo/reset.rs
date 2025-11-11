use aerosol::axum::Dep;
use axum::http::StatusCode;
use tracing::{error, info, instrument};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[instrument(skip(auth))]
pub async fn handle(auth: AuthenticatedToken, Dep(db): Dep<Postgres>) -> StatusCode {
    // Delete the authenticated org's cache data
    match db.cargo_cache_reset(&auth).await {
        Ok(()) => {
            info!("cache.reset.success");
            StatusCode::NO_CONTENT
        }
        Err(err) => {
            error!(error = ?err, "cache.reset.error");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use serde_json::json;
    use sqlx::PgPool;

    use crate::api::test_helpers::test_server;

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn resets_cache(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool.clone())
            .await
            .context("create test server")?;

        // Save some cache data via the API
        let request = json!({
            "package_name": "test-package",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "abc123",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "def456",
            "artifacts": [{
                "object_key": hex::encode((0..32).collect::<Vec<u8>>()),
                "path": "/path/to/artifact",
                "mtime_nanos": 123456789u128,
                "executable": false
            }]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Verify data exists
        let db = crate::db::Postgres { pool: pool.clone() };
        let count = sqlx::query!("SELECT COUNT(*) as count FROM cargo_package")
            .fetch_one(&db.pool)
            .await
            .context("query packages")?
            .count
            .unwrap_or(0);
        pretty_assert_eq!(count, 1);

        // Reset cache
        let response = server
            .post("/api/v1/cache/cargo/reset")
            .authorization_bearer(auth.token_alice().expose())
            .await;
        response.assert_status(StatusCode::NO_CONTENT);

        // Verify cache metadata is gone
        let count = sqlx::query!("SELECT COUNT(*) as count FROM cargo_package")
            .fetch_one(&db.pool)
            .await
            .context("query packages after reset")?
            .count
            .unwrap_or(0);
        pretty_assert_eq!(count, 0);

        let count = sqlx::query!("SELECT COUNT(*) as count FROM cargo_library_unit_build")
            .fetch_one(&db.pool)
            .await
            .context("query builds after reset")?
            .count
            .unwrap_or(0);
        pretty_assert_eq!(count, 0);

        // Note: cargo_object (CAS layer) is not deleted as it's shared across orgs
        // Only the org's cache metadata is removed

        Ok(())
    }
}
