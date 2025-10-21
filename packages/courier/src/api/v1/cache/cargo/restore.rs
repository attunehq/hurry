use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use client::courier::v1::cache::{CargoRestoreRequest, CargoRestoreResponse};
use color_eyre::eyre::Report;
use tap::Pipe;
use tracing::{error, info};

use crate::db::{CargoRestoreCacheRequest, Postgres};

#[tracing::instrument]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    Json(request): Json<CargoRestoreRequest>,
) -> CacheRestoreResponse {
    let request = CargoRestoreCacheRequest::builder()
        .package_name(request.package_name)
        .package_version(request.package_version)
        .target(request.target)
        .library_crate_compilation_unit_hash(request.library_crate_compilation_unit_hash)
        .maybe_build_script_compilation_unit_hash(request.build_script_compilation_unit_hash)
        .maybe_build_script_execution_unit_hash(request.build_script_execution_unit_hash)
        .build();

    match db.cargo_cache_restore(request).await {
        Ok(artifacts) if artifacts.is_empty() => {
            info!("cache.restore.miss");
            CacheRestoreResponse::NotFound
        }
        Ok(artifacts) => {
            info!("cache.restore.hit");
            CargoRestoreResponse::builder()
                .artifacts(artifacts)
                .build()
                .pipe(Json)
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
    Ok(Json<CargoRestoreResponse>),
    NotFound,
    Error(Report),
}

impl IntoResponse for CacheRestoreResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheRestoreResponse::Ok(json) => (StatusCode::OK, json).into_response(),
            CacheRestoreResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CacheRestoreResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
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

    use crate::api::test_helpers::test_blob;

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_after_save(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key1) = crate::api::test_helpers::test_blob(b"artifact_1_content");
        let (_, key2) = crate::api::test_helpers::test_blob(b"artifact_2_content");

        let save_request = json!({
            "package_name": "serde",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "abc123",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "content_abc123",
            "artifacts": [
                {
                    "object_key": key1,
                    "path": "libserde.rlib",
                    "mtime_nanos": 1234567890123456789u128,
                    "executable": false
                },
                {
                    "object_key": key2,
                    "path": "libserde.so",
                    "mtime_nanos": 1234567890987654321u128,
                    "executable": true
                }
            ]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "serde",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "abc123",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<serde_json::Value>();

        let expected = json!({
            "artifacts": [
                {
                    "object_key": key1,
                    "path": "libserde.rlib",
                    "mtime_nanos": 1234567890123456789u128,
                    "executable": false
                },
                {
                    "object_key": key2,
                    "path": "libserde.so",
                    "mtime_nanos": 1234567890987654321u128,
                    "executable": true
                }
            ]
        });

        pretty_assert_eq!(restore_response, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_nonexistent_cache(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let restore_request = json!({
            "package_name": "nonexistent",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "does_not_exist",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_with_build_script_hashes(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"proc_macro_artifact");

        let save_request = json!({
            "package_name": "proc-macro-crate",
            "package_version": "2.0.0",
            "target": "x86_64-apple-darwin",
            "library_crate_compilation_unit_hash": "lib_hash",
            "build_script_compilation_unit_hash": "build_comp_hash",
            "build_script_execution_unit_hash": "build_exec_hash",
            "content_hash": "full_content_hash",
            "artifacts": [
                {
                    "object_key": key,
                    "path": "libproc_macro_crate.rlib",
                    "mtime_nanos": 9876543210123456789u128,
                    "executable": false
                }
            ]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "proc-macro-crate",
            "package_version": "2.0.0",
            "target": "x86_64-apple-darwin",
            "library_crate_compilation_unit_hash": "lib_hash",
            "build_script_compilation_unit_hash": "build_comp_hash",
            "build_script_execution_unit_hash": "build_exec_hash"
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<serde_json::Value>();

        let expected = json!({
            "artifacts": [{
                "object_key": key,
                "path": "libproc_macro_crate.rlib",
                "mtime_nanos": 9876543210123456789u128,
                "executable": false
            }]
        });

        pretty_assert_eq!(restore_response, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_wrong_build_script_hash(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"crate_v1_artifact");

        let save_request = json!({
            "package_name": "crate-with-build",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "lib_hash",
            "build_script_compilation_unit_hash": "build_hash_v1",
            "build_script_execution_unit_hash": null,
            "content_hash": "content_v1",
            "artifacts": [
                {
                    "object_key": key,
                    "path": "libcrate.rlib",
                    "mtime_nanos": 1000000000000000000u128,
                    "executable": false
                }
            ]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "crate-with-build",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "lib_hash",
            "build_script_compilation_unit_hash": "build_hash_v2",
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_different_targets(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let targets = vec![
            "x86_64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ];

        let keyed_targets = targets
            .iter()
            .map(|target| (target, test_blob(format!("target_{target}").as_bytes()).1));
        for (i, (target, key)) in keyed_targets.clone().enumerate() {
            let save_request = json!({
                "package_name": "cross-platform-crate",
                "package_version": "1.0.0",
                "target": *target,
                "library_crate_compilation_unit_hash": format!("hash_{i}"),
                "build_script_compilation_unit_hash": null,
                "build_script_execution_unit_hash": null,
                "content_hash": format!("content_{i}"),
                "artifacts": [
                    {
                        "object_key": key,
                        "path": "libcross_platform_crate.rlib",
                        "mtime_nanos": 1000000000000000000u128 + i as u128,
                        "executable": false
                    }
                ]
            });

            let response = server
                .post("/api/v1/cache/cargo/save")
                .json(&save_request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        for (i, (target, key)) in keyed_targets.enumerate() {
            let restore_request = json!({
                "package_name": "cross-platform-crate",
                "package_version": "1.0.0",
                "target": *target,
                "library_crate_compilation_unit_hash": format!("hash_{i}"),
                "build_script_compilation_unit_hash": null,
                "build_script_execution_unit_hash": null
            });

            let response = server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request)
                .await;

            response.assert_status_ok();
            let restore_response = response.json::<serde_json::Value>();

            let expected = json!({
                "artifacts": [{
                    "object_key": key,
                    "path": "libcross_platform_crate.rlib",
                    "mtime_nanos": 1000000000000000000u128 + i as u128,
                    "executable": false
                }]
            });

            pretty_assert_eq!(restore_response, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_with_many_artifacts(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let mut keys = vec![];
        let artifacts = (0..50)
            .map(|i| {
                let (_, key) =
                    crate::api::test_helpers::test_blob(format!("artifact_{i}").as_bytes());
                keys.push(key.clone());
                json!({
                    "object_key": key,
                    "path": format!("artifact_{i}.o"),
                    "mtime_nanos": 1000000000000000000u128 + i as u128,
                    "executable": i % 3 == 0
                })
            })
            .collect::<Vec<_>>();

        let save_request = json!({
            "package_name": "large-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "large_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "large_content",
            "artifacts": artifacts
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "large-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "large_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<serde_json::Value>();

        let expected = json!({
            "artifacts": artifacts
        });

        pretty_assert_eq!(restore_response, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn concurrent_restores_same_cache(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, test_key) = crate::api::test_helpers::test_blob(b"concurrent_content");
        let save_request = json!({
            "package_name": "concurrent-test",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "concurrent_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "concurrent_content",
            "artifacts": [
                {
                    "object_key": test_key,
                    "path": "libconcurrent.rlib",
                    "mtime_nanos": 1111111111111111111u128,
                    "executable": false
                }
            ]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "concurrent-test",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "concurrent_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
            server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request),
        );

        let expected = json!({
            "artifacts": [{
                "object_key": test_key,
                "path": "libconcurrent.rlib",
                "mtime_nanos": 1111111111111111111u128,
                "executable": false
            }]
        });

        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status_ok();
            let restore_response = response.json::<serde_json::Value>();
            pretty_assert_eq!(restore_response, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_different_package_versions(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let versions = vec!["1.0.0", "1.0.1", "2.0.0"];

        let mut keys = vec![];
        for (i, version) in versions.iter().enumerate() {
            let (_, key) =
                crate::api::test_helpers::test_blob(format!("version_{version}").as_bytes());
            keys.push(key.clone());

            let save_request = json!({
                "package_name": "versioned-crate",
                "package_version": *version,
                "target": "x86_64-unknown-linux-gnu",
                "library_crate_compilation_unit_hash": format!("hash_{i}"),
                "build_script_compilation_unit_hash": null,
                "build_script_execution_unit_hash": null,
                "content_hash": format!("content_{i}"),
                "artifacts": [
                    {
                        "object_key": key,
                        "path": "libversioned_crate.rlib",
                        "mtime_nanos": 1000000000000000000u128 + i as u128,
                        "executable": false
                    }
                ]
            });

            let response = server
                .post("/api/v1/cache/cargo/save")
                .json(&save_request)
                .await;
            response.assert_status(StatusCode::CREATED);
        }

        for (i, version) in versions.iter().enumerate() {
            let restore_request = json!({
                "package_name": "versioned-crate",
                "package_version": *version,
                "target": "x86_64-unknown-linux-gnu",
                "library_crate_compilation_unit_hash": format!("hash_{i}"),
                "build_script_compilation_unit_hash": null,
                "build_script_execution_unit_hash": null
            });

            let response = server
                .post("/api/v1/cache/cargo/restore")
                .json(&restore_request)
                .await;

            response.assert_status_ok();
            let restore_response = response.json::<serde_json::Value>();

            let expected = json!({
                "artifacts": [{
                    "object_key": keys[i],
                    "path": "libversioned_crate.rlib",
                    "mtime_nanos": 1000000000000000000u128 + i as u128,
                    "executable": false
                }]
            });

            pretty_assert_eq!(restore_response, expected);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_preserves_mtime_precision(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let precise_mtime = 1234567890123456789u128;
        let (_, key) = crate::api::test_helpers::test_blob(b"precision_artifact");

        let save_request = json!({
            "package_name": "precision-test",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "precision_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "precision_content",
            "artifacts": [
                {
                    "object_key": key,
                    "path": "libprecision.rlib",
                    "mtime_nanos": precise_mtime,
                    "executable": false
                }
            ]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "precision-test",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "precision_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;

        response.assert_status_ok();
        let restore_response = response.json::<serde_json::Value>();

        let expected = json!({
            "artifacts": [{
                "object_key": key,
                "path": "libprecision.rlib",
                "mtime_nanos": precise_mtime,
                "executable": false
            }]
        });

        pretty_assert_eq!(restore_response, expected);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_wrong_package_name(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"wrong_name_test");

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": key,
                "path": "libtest.rlib",
                "mtime_nanos": 1000000000000000000u128,
                "executable": false
            }]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "wrong-name",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;
        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_wrong_package_version(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"wrong_version_test");

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": key,
                "path": "libtest.rlib",
                "mtime_nanos": 1000000000000000000u128,
                "executable": false
            }]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "test-crate",
            "package_version": "2.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;
        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_wrong_target(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"wrong_target_test");

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": key,
                "path": "libtest.rlib",
                "mtime_nanos": 1000000000000000000u128,
                "executable": false
            }]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "aarch64-apple-darwin",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;
        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_wrong_library_crate_hash(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = crate::api::test_helpers::test_blob(b"wrong_hash_test");

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "correct_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": key,
                "path": "libtest.rlib",
                "mtime_nanos": 1000000000000000000u128,
                "executable": false
            }]
        });

        let response = server
            .post("/api/v1/cache/cargo/save")
            .json(&save_request)
            .await;
        response.assert_status(StatusCode::CREATED);

        let restore_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "wrong_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null
        });

        let response = server
            .post("/api/v1/cache/cargo/restore")
            .json(&restore_request)
            .await;
        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }
}
