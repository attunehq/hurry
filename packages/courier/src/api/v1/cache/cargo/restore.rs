use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use derive_more::{Debug, From};
use serde::{Deserialize, Serialize};
use tap::Pipe;
use tracing::{error, info};

use crate::db::{CargoRestoreCacheRequest, Postgres};

use super::ArtifactFile;

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, From)]
pub struct RestoreResponse {
    #[debug("{:?}", self.artifacts.len())]
    pub artifacts: Vec<ArtifactFile>,
}

#[tracing::instrument]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    Json(request): Json<RestoreRequest>,
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
            RestoreResponse::from(
                artifacts
                    .into_iter()
                    .map(ArtifactFile::from)
                    .collect::<Vec<_>>(),
            )
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
    Ok(Json<RestoreResponse>),
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

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn restore_after_save(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

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
                    "object_key": "blake3_hash_1",
                    "path": "libserde.rlib",
                    "mtime_nanos": 1234567890123456789u128,
                    "executable": false
                },
                {
                    "object_key": "blake3_hash_2",
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
                    "object_key": "blake3_hash_1",
                    "path": "libserde.rlib",
                    "mtime_nanos": 1234567890123456789u128,
                    "executable": false
                },
                {
                    "object_key": "blake3_hash_2",
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
                    "object_key": "artifact_key",
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
                "object_key": "artifact_key",
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
                    "object_key": "key_v1",
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

        for (i, target) in targets.iter().enumerate() {
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
                        "object_key": format!("key_{target}"),
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

        for (i, target) in targets.iter().enumerate() {
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
                    "object_key": format!("key_{target}"),
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

        let artifacts = (0..50)
            .map(|i| {
                json!({
                    "object_key": format!("object_key_{i}"),
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
                    "object_key": "concurrent_key",
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
                "object_key": "concurrent_key",
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

        for (i, version) in versions.iter().enumerate() {
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
                        "object_key": format!("key_{version}"),
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
                    "object_key": format!("key_{version}"),
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
                    "object_key": "precision_key",
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
                "object_key": "precision_key",
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

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": "test_key",
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

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": "test_key",
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

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "test_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": "test_key",
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

        let save_request = json!({
            "package_name": "test-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "correct_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "test_content",
            "artifacts": [{
                "object_key": "test_key",
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
