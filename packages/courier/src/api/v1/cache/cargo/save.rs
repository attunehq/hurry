use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use clients::courier::v1::cache::CargoSaveRequest;
use color_eyre::eyre::Report;
use tracing::{error, info};

use crate::{auth::AuthenticatedToken, db::Postgres};

#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoSaveRequest>,
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
    use axum::http::StatusCode;
    use clients::courier::v1::cache::{ArtifactFile, CargoRestoreRequest, CargoSaveRequest};
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, test_server};

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn basic_save_flow(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key1) = crate::api::test_helpers::test_blob(b"serde_artifact_1");
        let (_, key2) = crate::api::test_helpers::test_blob(b"serde_artifact_2");

        let request = CargoSaveRequest::builder()
            .package_name("serde")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("abc123")
            .content_hash("content_abc123")
            .artifacts([
                ArtifactFile::builder()
                    .object_key(&key1)
                    .path("libserde.rlib")
                    .mtime_nanos(1234567890123456789u128)
                    .executable(false)
                    .build(),
                ArtifactFile::builder()
                    .object_key(&key2)
                    .path("libserde.so")
                    .mtime_nanos(1234567890987654321u128)
                    .executable(true)
                    .build(),
            ])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Verify database state
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        let restore_request = CargoRestoreRequest::builder()
            .package_name("serde")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("abc123")
            .build();

        let artifacts = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;
        let expected = vec![
            ArtifactFile::builder()
                .object_key(key1)
                .path("libserde.rlib")
                .mtime_nanos(1234567890123456789u128)
                .executable(false)
                .build(),
            ArtifactFile::builder()
                .object_key(key2)
                .path("libserde.so")
                .mtime_nanos(1234567890987654321u128)
                .executable(true)
                .build(),
        ];

        pretty_assert_eq!(artifacts, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn idempotent_saves(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key1) = crate::api::test_helpers::test_blob(b"idempotent_1");
        let (_, key2) = crate::api::test_helpers::test_blob(b"idempotent_2");

        let request = CargoSaveRequest::builder()
            .package_name("serde")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("abc123")
            .content_hash("content_abc123")
            .artifacts([
                ArtifactFile::builder()
                    .object_key(&key1)
                    .path("libserde.rlib")
                    .mtime_nanos(1234567890123456789u128)
                    .executable(false)
                    .build(),
                ArtifactFile::builder()
                    .object_key(&key2)
                    .path("libserde.so")
                    .mtime_nanos(1234567890987654321u128)
                    .executable(true)
                    .build(),
            ])
            .build();

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

        // Verify database state after idempotent saves
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        let restore_request = CargoRestoreRequest::builder()
            .package_name("serde")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("abc123")
            .build();

        let artifacts = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;
        let expected = vec![
            ArtifactFile::builder()
                .object_key(key1)
                .path("libserde.rlib")
                .mtime_nanos(1234567890123456789u128)
                .executable(false)
                .build(),
            ArtifactFile::builder()
                .object_key(key2)
                .path("libserde.so")
                .mtime_nanos(1234567890987654321u128)
                .executable(true)
                .build(),
        ];

        pretty_assert_eq!(artifacts, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_with_build_script_hashes(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"proc_macro_artifact");

        let request = CargoSaveRequest::builder()
            .package_name("proc-macro-crate")
            .package_version("2.0.0")
            .target("x86_64-apple-darwin")
            .library_crate_compilation_unit_hash("lib_hash")
            .build_script_compilation_unit_hash("build_comp_hash")
            .build_script_execution_unit_hash("build_exec_hash")
            .content_hash("full_content_hash")
            .artifacts([ArtifactFile::builder()
                .object_key(&key)
                .path("libproc_macro_crate.rlib")
                .mtime_nanos(9876543210123456789u128)
                .executable(false)
                .build()])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Verify database state
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        let restore_request = CargoRestoreRequest::builder()
            .package_name("proc-macro-crate")
            .package_version("2.0.0")
            .target("x86_64-apple-darwin")
            .library_crate_compilation_unit_hash("lib_hash")
            .build_script_compilation_unit_hash("build_comp_hash")
            .build_script_execution_unit_hash("build_exec_hash")
            .build();

        let artifacts = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;
        let expected = vec![
            ArtifactFile::builder()
                .object_key(key)
                .path("libproc_macro_crate.rlib")
                .mtime_nanos(9876543210123456789u128)
                .executable(false)
                .build(),
        ];

        pretty_assert_eq!(artifacts, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_multiple_packages(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let packages = [("serde", "1.0.0"), ("tokio", "1.35.0"), ("axum", "0.7.0")];
        let keyed_packages = packages.iter().enumerate().map(|(i, (name, version))| {
            (
                name,
                version,
                test_blob(format!("package_{i}").as_bytes()).1,
            )
        });

        for (i, (name, version, key)) in keyed_packages.clone().enumerate() {
            let request = CargoSaveRequest::builder()
                .package_name(*name)
                .package_version(*version)
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash(format!("hash_{i}"))
                .content_hash(format!("content_{i}"))
                .artifacts([ArtifactFile::builder()
                    .object_key(key)
                    .path(format!("lib{name}.rlib"))
                    .mtime_nanos(1000000000000000000u128 + i as u128)
                    .executable(false)
                    .build()])
                .build();

            let response = server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        // Verify all packages were saved correctly
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        for (i, (name, version, key)) in keyed_packages.enumerate() {
            let restore_request = CargoRestoreRequest::builder()
                .package_name(*name)
                .package_version(*version)
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash(format!("hash_{i}"))
                .build();

            let artifacts = db
                .cargo_cache_restore(&alice_validated, restore_request)
                .await?;
            let expected = vec![
                ArtifactFile::builder()
                    .object_key(key)
                    .path(format!("lib{name}.rlib"))
                    .mtime_nanos(1000000000000000000u128 + i as u128)
                    .executable(false)
                    .build(),
            ];
            pretty_assert_eq!(artifacts, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_same_package_different_targets(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let targets = [
            "x86_64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ];

        let keyed_targets = targets
            .iter()
            .map(|target| (target, test_blob(format!("target_{target}").as_bytes()).1));

        for (i, (target, key)) in keyed_targets.clone().enumerate() {
            let request = CargoSaveRequest::builder()
                .package_name("serde")
                .package_version("1.0.0")
                .target(*target)
                .library_crate_compilation_unit_hash(format!("hash_{i}"))
                .content_hash(format!("content_{i}"))
                .artifacts([ArtifactFile::builder()
                    .object_key(key)
                    .path("libserde.rlib")
                    .mtime_nanos(1234567890000000000u128 + i as u128)
                    .executable(false)
                    .build()])
                .build();

            let response = server
                .post("/api/v1/cache/cargo/save")
                .authorization_bearer(auth.token_alice().expose())
                .json(&request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        // Verify all targets were saved correctly for the same package
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        for (i, (target, key)) in keyed_targets.enumerate() {
            let restore_request = CargoRestoreRequest::builder()
                .package_name("serde")
                .package_version("1.0.0")
                .target(*target)
                .library_crate_compilation_unit_hash(format!("hash_{i}"))
                .build();

            let artifacts = db
                .cargo_cache_restore(&alice_validated, restore_request)
                .await?;
            let expected = vec![
                ArtifactFile::builder()
                    .object_key(key)
                    .path("libserde.rlib")
                    .mtime_nanos(1234567890000000000u128 + i as u128)
                    .executable(false)
                    .build(),
            ];
            pretty_assert_eq!(artifacts, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_reuses_existing_objects(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, shared_object_key) = crate::api::test_helpers::test_blob(b"shared_object");

        let request1 = CargoSaveRequest::builder()
            .package_name("dep-a")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash_a")
            .content_hash("content_a")
            .artifacts([ArtifactFile::builder()
                .object_key(&shared_object_key)
                .path("liba.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response1 = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request1)
            .await;
        response1.assert_status(StatusCode::CREATED);

        let request2 = CargoSaveRequest::builder()
            .package_name("dep-b")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash_b")
            .content_hash("content_b")
            .artifacts([ArtifactFile::builder()
                .object_key(&shared_object_key)
                .path("libb.rlib")
                .mtime_nanos(2000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response2 = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request2)
            .await;
        response2.assert_status(StatusCode::CREATED);

        // Verify both packages can restore with shared object
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");

        let restore_a = CargoRestoreRequest::builder()
            .package_name("dep-a")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash_a")
            .build();

        let artifacts_a = db.cargo_cache_restore(&alice_validated, restore_a).await?;
        let expected_a = vec![
            ArtifactFile::builder()
                .object_key(&shared_object_key)
                .path("liba.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build(),
        ];
        pretty_assert_eq!(artifacts_a, expected_a);

        let restore_b = CargoRestoreRequest::builder()
            .package_name("dep-b")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash_b")
            .build();

        let artifacts_b = db.cargo_cache_restore(&alice_validated, restore_b).await?;
        let expected_b = vec![
            ArtifactFile::builder()
                .object_key(&shared_object_key)
                .path("libb.rlib")
                .mtime_nanos(2000000000000000000u128)
                .executable(false)
                .build(),
        ];
        pretty_assert_eq!(artifacts_b, expected_b);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_with_many_artifacts(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let artifacts = (0..20)
            .map(|i| {
                let (_, key) = test_blob(format!("artifact_{i}").as_bytes());
                ArtifactFile::builder()
                    .object_key(key)
                    .path(format!("artifact_{i}.o"))
                    .mtime_nanos(1000000000000000000u128 + i as u128)
                    .executable(i % 3 == 0)
                    .build()
            })
            .collect::<Vec<_>>();

        let request = CargoSaveRequest::builder()
            .package_name("large-crate")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("large_hash")
            .content_hash("large_content")
            .artifacts(artifacts)
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request)
            .await;
        response.assert_status(StatusCode::CREATED);

        // Verify all artifacts were saved correctly
        let restore_request = CargoRestoreRequest::builder()
            .package_name("large-crate")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("large_hash")
            .build();

        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        let artifacts = db
            .cargo_cache_restore(&alice_validated, restore_request)
            .await?;
        let expected = artifacts
            .iter()
            .enumerate()
            .map(|(i, artifact)| {
                ArtifactFile::builder()
                    .object_key(&artifact.object_key)
                    .path(format!("artifact_{i}.o"))
                    .mtime_nanos(1000000000000000000u128 + i as u128)
                    .executable(i % 3 == 0)
                    .build()
            })
            .collect::<Vec<_>>();

        pretty_assert_eq!(artifacts, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn concurrent_saves_different_packages(pool: PgPool) -> Result<()> {
        let db = crate::db::Postgres { pool: pool.clone() };
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let requests = (0..10)
            .map(|i| {
                let (_, key) = test_blob(format!("concurrent_{i}").as_bytes());
                CargoSaveRequest::builder()
                    .package_name(format!("crate-{i}"))
                    .package_version("1.0.0")
                    .target("x86_64-unknown-linux-gnu")
                    .library_crate_compilation_unit_hash(format!("hash_{i}"))
                    .content_hash(format!("content_{i}"))
                    .artifacts([ArtifactFile::builder()
                        .object_key(key)
                        .path(format!("libcrate_{i}.rlib"))
                        .mtime_nanos(1000000000000000000u128 + i as u128)
                        .executable(false)
                        .build()])
                    .build()
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

        // Verify all concurrent saves were correctly stored
        let alice_validated = db
            .validate(auth.token_alice())
            .await
            .expect("validate token")
            .expect("token must exist");
        for i in 0..10 {
            let restore_request = CargoRestoreRequest::builder()
                .package_name(format!("crate-{i}"))
                .package_version("1.0.0")
                .target("x86_64-unknown-linux-gnu")
                .library_crate_compilation_unit_hash(format!("hash_{i}"))
                .build();

            let artifacts = db
                .cargo_cache_restore(&alice_validated, restore_request)
                .await?;
            let expected = artifacts
                .iter()
                .map(|artifact| {
                    ArtifactFile::builder()
                        .object_key(artifact.object_key.clone())
                        .path(format!("libcrate_{i}.rlib"))
                        .mtime_nanos(1000000000000000000u128 + i as u128)
                        .executable(false)
                        .build()
                })
                .collect::<Vec<_>>();
            pretty_assert_eq!(artifacts, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_content_hash_mismatch_fails(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key1) = crate::api::test_helpers::test_blob(b"content_v1");
        let (_, key2) = crate::api::test_helpers::test_blob(b"content_v2");

        let request1 = CargoSaveRequest::builder()
            .package_name("test-crate")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("same_hash")
            .content_hash("content_v1")
            .artifacts([ArtifactFile::builder()
                .object_key(key1)
                .path("libtest.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response1 = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request1)
            .await;
        response1.assert_status(StatusCode::CREATED);

        // Try to save with same unit hashes but different content_hash
        let request2 = CargoSaveRequest::builder()
            .package_name("test-crate")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("same_hash")
            .content_hash("content_v2")
            .artifacts([ArtifactFile::builder()
                .object_key(key2)
                .path("libtest.rlib")
                .mtime_nanos(2000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response2 = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice().expose())
            .json(&request2)
            .await;
        response2.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_missing_auth_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"artifact content");
        let request = CargoSaveRequest::builder()
            .package_name("test-pkg")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash")
            .content_hash("content")
            .artifacts([ArtifactFile::builder()
                .object_key(key)
                .path("lib.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response = server.post("/api/v1/cache/cargo/save").json(&request).await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn save_invalid_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"artifact content");
        let request = CargoSaveRequest::builder()
            .package_name("test-pkg")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash")
            .content_hash("content")
            .artifacts([ArtifactFile::builder()
                .object_key(key)
                .path("lib.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

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

        let (_, key) = test_blob(b"artifact content");
        let request = CargoSaveRequest::builder()
            .package_name("test-pkg")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("hash")
            .content_hash("content")
            .artifacts([ArtifactFile::builder()
                .object_key(key)
                .path("lib.rlib")
                .mtime_nanos(1000000000000000000u128)
                .executable(false)
                .build()])
            .build();

        let response = server
            .post("/api/v1/cache/cargo/save")
            .authorization_bearer(auth.token_alice_revoked().expose())
            .json(&request)
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }
}
