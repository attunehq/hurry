use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::CargoSaveRequest2;
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoSaveRequest2>,
) -> CacheSaveResponse {
    match db.cargo_cache_save(&auth, request).await {
        Ok(()) => {
            info!("cache.save.created");
            CacheSaveResponse::Created
        }
        Err(err) => {
            error!(error = ?err, "cache.save.error");
            CacheSaveResponse::Error(err)
        }
    }
}

#[derive(Debug)]
pub enum CacheSaveResponse {
    Created,
    Error(Report),
}

impl IntoResponse for CacheSaveResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheSaveResponse::Created => StatusCode::CREATED.into_response(),
            CacheSaveResponse::Error(error) => {
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
        cache::{CargoRestoreRequest2, CargoSaveRequest2, CargoSaveUnitRequest, SavedUnitCacheKey},
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
    async fn basic_save_flow(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key, unit) = test_saved_unit("serde", "v1");

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

        // Verify it was stored
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        let restore_request = CargoRestoreRequest2::new([&cache_key]);
        let restored = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;

        let expected = HashSet::from([(cache_key, unit)]);
        let restored = restored.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn idempotent_saves(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (cache_key, unit) = test_saved_unit("serde", "v1");

        let request = CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
            .key(&cache_key)
            .unit(&unit)
            .build()]);

        let response1 = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response1.assert_status(StatusCode::CREATED);

        let response2 = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response2.assert_status(StatusCode::CREATED);

        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");

        let restore_request = CargoRestoreRequest2::new([&cache_key]);
        let restored = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;

        let expected = HashSet::from([(cache_key, unit)]);
        let restored = restored.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_multiple_packages(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let packages = ["serde", "tokio", "axum"];
        let units = packages
            .iter()
            .map(|name| test_saved_unit(name, "v1"))
            .map(|(key, unit)| CargoSaveUnitRequest::builder().key(key).unit(unit).build())
            .collect::<Vec<_>>();

        let request = CargoSaveRequest2::new(units);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");

        let expected = packages
            .iter()
            .map(|name| test_saved_unit(name, "v1"))
            .collect::<HashSet<_>>();

        let restore_request =
            CargoRestoreRequest2::new(expected.iter().map(|(key, _)| key).collect::<Vec<_>>());
        let restored = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;

        let restored = restored.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_same_package_different_hashes(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let units = ["v1", "v2", "v3"]
            .iter()
            .map(|suffix| test_saved_unit("serde", suffix))
            .map(|(key, unit)| CargoSaveUnitRequest::builder().key(key).unit(unit).build())
            .collect::<Vec<_>>();

        let request = CargoSaveRequest2::new(units);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");

        let expected = ["v1", "v2", "v3"]
            .iter()
            .map(|suffix| test_saved_unit("serde", suffix))
            .collect::<HashSet<_>>();

        let restore_request =
            CargoRestoreRequest2::new(expected.iter().map(|(key, _)| key).collect::<Vec<_>>());
        let restored = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;

        let restored = restored.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn concurrent_saves_different_packages(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let packages = (0..10).map(|i| format!("crate-{i}")).collect::<Vec<_>>();
        let requests = packages
            .iter()
            .map(|name| {
                let (key, unit) = test_saved_unit(name, "v1");
                CargoSaveRequest2::new([CargoSaveUnitRequest::builder()
                    .key(key)
                    .unit(unit)
                    .build()])
            })
            .collect::<Vec<_>>();

        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[0]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[1]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[2]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[3]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[4]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[5]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[6]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[7]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[8]),
            server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&requests[9]),
        );

        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");

        let expected = (0..10)
            .map(|i| test_saved_unit(&format!("crate-{i}"), "v1"))
            .collect::<HashSet<_>>();

        let restore_request =
            CargoRestoreRequest2::new(expected.iter().map(|(key, _)| key).collect::<Vec<_>>());
        let restored = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;

        let restored = restored.into_iter().collect::<HashSet<_>>();
        pretty_assert_eq!(restored, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_missing_auth_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (key, unit) = test_saved_unit("test", "v1");
        let request =
            CargoSaveRequest2::new([CargoSaveUnitRequest::builder().key(key).unit(unit).build()]);

        let response = server.post("/api/v1/cache/cargo/save").json(&request).await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_invalid_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (key, unit) = test_saved_unit("test", "v1");
        let request =
            CargoSaveRequest2::new([CargoSaveUnitRequest::builder().key(key).unit(unit).build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer("invalid-token-that-does-not-exist")
            .json(&request)
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_revoked_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (key, unit) = test_saved_unit("test", "v1");
        let request =
            CargoSaveRequest2::new([CargoSaveUnitRequest::builder().key(key).unit(unit).build()]);

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice_revoked().expose())
            .json(&request)
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }
}
