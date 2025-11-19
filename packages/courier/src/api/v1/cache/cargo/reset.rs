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
    use clients::courier::v1::{
        DiskPath, Fingerprint, LibraryCrateUnitPlan, LibraryFiles, SavedFile, SavedUnit,
        SavedUnitHash, UnitPlanInfo,
        cache::{CargoSaveRequest2, CargoSaveUnitRequest, SavedUnitCacheKey},
    };
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, test_server};

    fn test_saved_unit(name: &str, hash_suffix: &str) -> (SavedUnitCacheKey, SavedUnit) {
        let unit_hash = SavedUnitHash::new(format!("{name}_{hash_suffix}"));
        let (_, key1) = test_blob(format!("{name}_output_1").as_bytes());
        let (_, key2) = test_blob(format!("{name}_dep_info").as_bytes());
        let (_, key3) = test_blob(format!("{name}_encoded_dep_info").as_bytes());

        let cache_key = SavedUnitCacheKey::builder().unit(&unit_hash).build();

        let unit = SavedUnit::LibraryCrate(
            LibraryFiles::builder()
                .output_files(vec![
                    SavedFile::builder()
                        .object_key(key1)
                        .path(DiskPath::new(format!("lib{name}.rlib")))
                        .executable(false)
                        .build(),
                ])
                .fingerprint(Fingerprint::new(format!("{name}_fingerprint")))
                .dep_info_file(key2)
                .encoded_dep_info_file(key3)
                .build(),
            LibraryCrateUnitPlan::builder()
                .info(
                    UnitPlanInfo::builder()
                        .unit_hash(&unit_hash)
                        .package_name(name)
                        .crate_name(name)
                        .target_arch("x86_64-unknown-linux-gnu")
                        .build(),
                )
                .src_path(DiskPath::new("src/lib.rs"))
                .outputs(vec![DiskPath::new(format!("lib{name}.rlib"))])
                .build(),
        );

        (cache_key, unit)
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn resets_cache(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key, unit) = test_saved_unit("test-package", "v1");

        let request = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&cache_key)
            .unit(&unit)
            .build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Verify data exists
        let count = sqlx::query!("SELECT COUNT(*) as count FROM cargo_saved_unit")
            .fetch_one(&db.pool)
            .await
            .context("query cargo_saved_unit")?
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
        let count = sqlx::query!("SELECT COUNT(*) as count FROM cargo_saved_unit")
            .fetch_one(&db.pool)
            .await
            .context("query cargo_saved_unit after reset")?
            .count
            .unwrap_or(0);
        pretty_assert_eq!(count, 0);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn org_reset_only_deletes_own_data(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (key_a, unit_a) = test_saved_unit("pkg-a", "v1");
        let (key_b, unit_b) = test_saved_unit("pkg-b", "v1");

        // Org A saves cache
        let save_a = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&key_a)
            .unit(unit_a)
            .build()]);

        server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_a)
            .await
            .assert_status(StatusCode::CREATED);

        // Org B saves cache
        let save_b = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&key_b)
            .unit(unit_b)
            .build()]);

        server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_charlie().expose())
            .json(&save_b)
            .await
            .assert_status(StatusCode::CREATED);

        // Org A resets their cache
        server
            .post("/api/v1/cache/cargo/reset")
            .authorization_bearer(auth.token_alice().expose())
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Org A's cache should be gone
        use clients::courier::v1::cache::CargoRestoreRequest2;

        let restore_a = CargoRestoreRequest2::new([&key_a]);

        server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&restore_a)
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // Org B's cache should still exist
        let restore_b = CargoRestoreRequest2::new([&key_b]);

        server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_charlie().expose())
            .json(&restore_b)
            .await
            .assert_status_ok();

        Ok(())
    }
}
