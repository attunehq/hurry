use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::db::Postgres;

use super::ArtifactFile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreResponse {
    pub artifacts: Vec<ArtifactFile>,
}

#[tracing::instrument]
pub async fn handle(
    Dep(db): Dep<Postgres>,
    Json(request): Json<RestoreRequest>,
) -> CacheRestoreResponse {
    match restore_from_database(&db, request).await {
        Ok(response) => {
            info!("cache.restore.success");
            CacheRestoreResponse::Ok(Json(response))
        }
        Err(err) => {
            error!(error = ?err, "cache.restore.error");
            CacheRestoreResponse::Error(err)
        }
    }
}

async fn restore_from_database(
    db: &Postgres,
    request: RestoreRequest,
) -> Result<RestoreResponse, Report> {
    let mut tx = db.pool.begin().await?;
    let unit_builds = sqlx::query!(
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
    .fetch_all(&mut *tx)
    .await?;

    let unit_to_restore = match unit_builds.split_first() {
        Some((first, rest)) => {
            if !rest.is_empty() {
                if rest
                    .iter()
                    .all(|unit| unit.content_hash == first.content_hash)
                {
                    first.id
                } else {
                    warn!(?unit_builds, "multiple matching library unit builds found");
                    return Ok(RestoreResponse { artifacts: vec![] });
                }
            } else {
                first.id
            }
        }
        None => {
            debug!("no matching library unit build found");
            return Ok(RestoreResponse { artifacts: vec![] });
        }
    };

    let objects = sqlx::query!(
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
        unit_to_restore
    )
    .fetch_all(&mut *tx)
    .await?;

    let artifacts = objects
        .into_iter()
        .map(|obj| {
            let mtime_nanos = obj.mtime.to_u128().unwrap_or_else(|| {
                error!("failed to convert mtime to u128");
                0
            });
            ArtifactFile {
                object_key: obj.key,
                path: obj.path,
                mtime_nanos,
                executable: obj.executable,
            }
        })
        .collect::<Vec<_>>();

    Ok(RestoreResponse { artifacts })
}

#[derive(Debug)]
pub enum CacheRestoreResponse {
    Ok(Json<RestoreResponse>),
    Error(Report),
}

impl IntoResponse for CacheRestoreResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CacheRestoreResponse::Ok(json) => (StatusCode::OK, json).into_response(),
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

        response.assert_status_ok();
        let restore_response = response.json::<serde_json::Value>();

        let expected = json!({
            "artifacts": []
        });

        pretty_assert_eq!(restore_response, expected);

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

        response.assert_status_ok();
        let restore_response = response.json::<serde_json::Value>();

        let expected = json!({
            "artifacts": []
        });

        pretty_assert_eq!(restore_response, expected);

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
}
