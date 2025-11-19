use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::{CargoRestoreRequest2, CargoRestoreResponseTransport};
use color_eyre::eyre::Report;
use tap::Pipe;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoRestoreRequest2>,
) -> CacheRestoreResponse {
    match db.cargo_cache_restore(&auth, request).await {
        Ok(artifacts) if artifacts.is_empty() => {
            info!("cache.restore.miss");
            CacheRestoreResponse::NotFound
        }
        Ok(artifacts) => {
            info!("cache.restore.hit");
            artifacts
                .into_iter()
                .collect::<CargoRestoreResponseTransport>()
                .pipe(CacheRestoreResponse::Ok)
        }
        Err(err) => {
            error!(error = ?err, "cache.restore.error");
            CacheRestoreResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CacheRestoreResponse {
    Ok(CargoRestoreResponseTransport),
    NotFound,
    Error(Report),
}

impl IntoResponse for CacheRestoreResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheRestoreResponse::Ok(body) => (StatusCode::OK, Json(body)).into_response(),
            CacheRestoreResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CacheRestoreResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use axum::http::StatusCode;
    use clients::courier::v1::{
        DiskPath, Fingerprint, LibraryCrateUnitPlan, LibraryFiles, SavedFile, SavedUnit,
        SavedUnitHash, UnitPlanInfo,
        cache::{
            CargoRestoreRequest2, CargoRestoreResponseTransport, CargoSaveRequest2,
            CargoSaveUnitRequest, SavedUnitCacheKey,
        },
    };
    use color_eyre::{Result, Section, SectionExt, eyre::Context};
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
    async fn restore_after_save(pool: PgPool) -> Result<()> {
        let _ = color_eyre::install();
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key, unit) = test_saved_unit("serde", "v1");

        let save_request = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&cache_key)
            .unit(&unit)
            .build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = CargoRestoreRequest2::new([&cache_key]);
        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let body = response.text();
        let restored = serde_json::from_str::<CargoRestoreResponseTransport>(&body)
            .with_section(|| body.header("Response:"))?;

        let expected = HashSet::from([(cache_key.clone(), unit.clone())]);
        let restored = restored.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn restore_nonexistent_cache(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let cache_key = SavedUnitCacheKey::builder()
            .unit(SavedUnitHash::new("nonexistent"))
            .build();
        let restore_request = CargoRestoreRequest2::new([cache_key]);

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn restore_multiple_units(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let packages = ["serde", "tokio", "axum"];
        let units = packages
            .iter()
            .map(|name| test_saved_unit(name, "v1"))
            .collect::<Vec<_>>();

        let save_request = CargoSaveRequest2::new(
            units
                .iter()
                .map(|(key, unit)| CargoSaveUnitRequest::builder().key(key).unit(unit).build())
                .collect::<Vec<_>>(),
        );

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request =
            CargoRestoreRequest2::new(units.iter().map(|(key, _)| key).collect::<Vec<_>>());

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<CargoRestoreResponseTransport>();

        let expected = units.into_iter().collect::<HashSet<_>>();
        let restored = restore_response.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn restore_partial_miss(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key1, unit1) = test_saved_unit("serde", "v1");
        let cache_key2 = SavedUnitCacheKey::builder()
            .unit(SavedUnitHash::new("nonexistent"))
            .build();

        let save_request = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&cache_key1)
            .unit(&unit1)
            .build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = CargoRestoreRequest2::new([&cache_key1, &cache_key2]);

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<CargoRestoreResponseTransport>();

        let expected = HashSet::from([(cache_key1, unit1)]);
        let restored = restore_response.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn concurrent_restores_same_cache(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key, unit) = test_saved_unit("concurrent", "v1");

        let save_request = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&cache_key)
            .unit(&unit)
            .build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = CargoRestoreRequest2::new([&cache_key]);

        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .authorization_bearer(auth.token_alice().expose())
                .json(&restore_request),
        );

        let expected = HashSet::from([(cache_key.clone(), unit.clone())]);
        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status_ok();
            let restore_response = response.json::<CargoRestoreResponseTransport>();
            let restored = restore_response.into_iter().collect::<HashSet<_>>();
            pretty_assert_eq!(restored, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn org_cannot_restore_other_orgs_cache(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key, unit) = test_saved_unit("private", "v1");

        // Org A (Alice's org) saves cache
        let save_request = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&cache_key)
            .unit(&unit)
            .build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Org B (Charlie's org) tries to restore it
        let restore_request = CargoRestoreRequest2::new([&cache_key]);

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_charlie().expose())
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        // Org A can still restore it
        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice().expose())
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<CargoRestoreResponseTransport>();

        let expected = HashSet::from([(cache_key, unit)]);
        let restored = restore_response.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn restore_missing_auth_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let cache_key = SavedUnitCacheKey::builder()
            .unit(SavedUnitHash::new("test"))
            .build();
        let restore_request = CargoRestoreRequest2::new([cache_key]);

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn restore_invalid_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let cache_key = SavedUnitCacheKey::builder()
            .unit(SavedUnitHash::new("test"))
            .build();
        let restore_request = CargoRestoreRequest2::new([cache_key]);

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer("invalid-token-that-does-not-exist")
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn restore_revoked_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let cache_key = SavedUnitCacheKey::builder()
            .unit(SavedUnitHash::new("test"))
            .build();
        let restore_request = CargoRestoreRequest2::new([cache_key]);

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .authorization_bearer(auth.token_alice_revoked().expose())
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }
}
