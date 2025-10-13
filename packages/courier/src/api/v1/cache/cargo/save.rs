use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::db::Postgres;

use super::ArtifactFile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
    pub content_hash: String,
    pub artifacts: Vec<ArtifactFile>,
}

#[tracing::instrument]
pub async fn handle(Dep(db): Dep<Postgres>, Json(request): Json<SaveRequest>) -> CacheSaveResponse {
    match save_to_database(&db, request).await {
        Ok(()) => {
            info!("cache.save.success");
            CacheSaveResponse::Created
        }
        Err(err) => {
            error!(error = ?err, "cache.save.error");
            CacheSaveResponse::Error(err)
        }
    }
}

async fn save_to_database(db: &Postgres, request: SaveRequest) -> Result<(), Report> {
    let mut tx = db.pool.begin().await?;

    let package_id = match sqlx::query!(
        "SELECT id FROM cargo_package WHERE name = $1 AND version = $2",
        request.package_name,
        request.package_version
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        Some(row) => row.id,
        None => {
            sqlx::query!(
                "INSERT INTO cargo_package (name, version) VALUES ($1, $2) RETURNING id",
                request.package_name,
                request.package_version
            )
            .fetch_one(&mut *tx)
            .await?
            .id
        }
    };

    match sqlx::query!(
        r#"
        SELECT content_hash
        FROM cargo_library_unit_build
        WHERE
            package_id = $1
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
    .await?
    {
        Some(row) => {
            if row.content_hash != request.content_hash {
                error!(expected = ?row.content_hash, actual = ?request.content_hash, "content hash mismatch");
            }
        }
        None => {
            let library_unit_build_id = sqlx::query!(
                r#"
                INSERT INTO cargo_library_unit_build (
                    package_id,
                    target,
                    library_crate_compilation_unit_hash,
                    build_script_compilation_unit_hash,
                    build_script_execution_unit_hash,
                    content_hash
                ) VALUES ($1, $2, $3, $4, $5, $6)
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
            .await?
            .id;

            for artifact in request.artifacts {
                let object_id = match sqlx::query!(
                    "SELECT id FROM cargo_object WHERE key = $1",
                    artifact.object_key
                )
                .fetch_optional(&mut *tx)
                .await?
                {
                    Some(row) => row.id,
                    None => {
                        sqlx::query!(
                            "INSERT INTO cargo_object (key) VALUES ($1) RETURNING id",
                            artifact.object_key
                        )
                        .fetch_one(&mut *tx)
                        .await?
                        .id
                    }
                };

                let mtime_numeric = bigdecimal::BigDecimal::from(artifact.mtime_nanos);

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
                    mtime_numeric,
                    artifact.executable
                )
                .execute(&mut *tx)
                .await?;
            }
        }
    };

    tx.commit().await?;

    Ok(())
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
    use color_eyre::{Result, eyre::Context};
    use serde_json::json;
    use sqlx::PgPool;

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn basic_save_flow(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let request = json!({
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

        let response = server.post("/api/v1/cache/cargo/save").json(&request).await;
        response.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn idempotent_saves(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let request = json!({
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

        let response1 = server.post("/api/v1/cache/cargo/save").json(&request).await;
        response1.assert_status(StatusCode::CREATED);

        let response2 = server.post("/api/v1/cache/cargo/save").json(&request).await;
        response2.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn save_with_build_script_hashes(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let request = json!({
            "package_name": "proc-macro-crate",
            "package_version": "2.0.0",
            "target": "x86_64-apple-darwin",
            "library_crate_compilation_unit_hash": "lib_hash",
            "build_script_compilation_unit_hash": "build_comp_hash",
            "build_script_execution_unit_hash": "build_exec_hash",
            "content_hash": "full_content_hash",
            "artifacts": [{
                "object_key": "artifact_key",
                "path": "libproc_macro_crate.rlib",
                "mtime_nanos": 9876543210123456789u128,
                "executable": false
            }]
        });

        let response = server.post("/api/v1/cache/cargo/save").json(&request).await;
        response.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn save_multiple_packages(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let packages = [("serde", "1.0.0"), ("tokio", "1.35.0"), ("axum", "0.7.0")];
        for (i, (name, version)) in packages.iter().enumerate() {
            let request = json!({
                "package_name": name,
                "package_version": version,
                "target": "x86_64-unknown-linux-gnu",
                "library_crate_compilation_unit_hash": format!("hash_{i}"),
                "build_script_compilation_unit_hash": null,
                "build_script_execution_unit_hash": null,
                "content_hash": format!("content_{i}"),
                "artifacts": [{
                    "object_key": format!("key_{i}"),
                    "path": format!("lib{name}.rlib"),
                    "mtime_nanos": 1000000000000000000u128 + i as u128,
                    "executable": false
                }]
            });

            let response = server.post("/api/v1/cache/cargo/save").json(&request).await;
            response.assert_status(StatusCode::CREATED);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn save_same_package_different_targets(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let targets = [
            "x86_64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ];

        for (i, target) in targets.iter().enumerate() {
            let request = json!({
                "package_name": "serde",
                "package_version": "1.0.0",
                "target": target,
                "library_crate_compilation_unit_hash": format!("hash_{i}"),
                "build_script_compilation_unit_hash": null,
                "build_script_execution_unit_hash": null,
                "content_hash": format!("content_{i}"),
                "artifacts": [{
                    "object_key": format!("key_{target}"),
                    "path": "libserde.rlib",
                    "mtime_nanos": 1234567890000000000u128 + i as u128,
                    "executable": false
                }]
            });

            let response = server.post("/api/v1/cache/cargo/save").json(&request).await;
            response.assert_status(StatusCode::CREATED);
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn save_reuses_existing_objects(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let shared_object_key = "shared_blake3_hash";

        let request1 = json!({
            "package_name": "dep-a",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "hash_a",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "content_a",
            "artifacts": [{
                "object_key": shared_object_key,
                "path": "liba.rlib",
                "mtime_nanos": 1000000000000000000u128,
                "executable": false
            }]
        });

        let response1 = server
            .post("/api/v1/cache/cargo/save")
            .json(&request1)
            .await;
        response1.assert_status(StatusCode::CREATED);

        let request2 = json!({
            "package_name": "dep-b",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "hash_b",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "content_b",
            "artifacts": [{
                "object_key": shared_object_key,
                "path": "libb.rlib",
                "mtime_nanos": 2000000000000000000u128,
                "executable": false
            }]
        });

        let response2 = server
            .post("/api/v1/cache/cargo/save")
            .json(&request2)
            .await;
        response2.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn save_with_many_artifacts(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let artifacts = (0..20)
            .map(|i| {
                json!({
                    "object_key": format!("object_key_{i}"),
                    "path": format!("artifact_{i}.o"),
                    "mtime_nanos": 1000000000000000000u128 + i as u128,
                    "executable": i % 3 == 0
                })
            })
            .collect::<Vec<_>>();

        let request = json!({
            "package_name": "large-crate",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "library_crate_compilation_unit_hash": "large_hash",
            "build_script_compilation_unit_hash": null,
            "build_script_execution_unit_hash": null,
            "content_hash": "large_content",
            "artifacts": artifacts
        });

        let response = server.post("/api/v1/cache/cargo/save").json(&request).await;
        response.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn concurrent_saves_different_packages(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let requests = (0..10)
            .map(|i| {
                json!({
                    "package_name": format!("crate-{i}"),
                    "package_version": "1.0.0",
                    "target": "x86_64-unknown-linux-gnu",
                    "library_crate_compilation_unit_hash": format!("hash_{i}"),
                    "build_script_compilation_unit_hash": null,
                    "build_script_execution_unit_hash": null,
                    "content_hash": format!("content_{i}"),
                    "artifacts": [{
                        "object_key": format!("key_{i}"),
                        "path": format!("libcrate_{i}.rlib"),
                        "mtime_nanos": 1000000000000000000u128 + i as u128,
                        "executable": false
                    }]
                })
            })
            .collect::<Vec<_>>();

        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server.post("/api/v1/cache/cargo/save").json(&requests[0]),
            server.post("/api/v1/cache/cargo/save").json(&requests[1]),
            server.post("/api/v1/cache/cargo/save").json(&requests[2]),
            server.post("/api/v1/cache/cargo/save").json(&requests[3]),
            server.post("/api/v1/cache/cargo/save").json(&requests[4]),
            server.post("/api/v1/cache/cargo/save").json(&requests[5]),
            server.post("/api/v1/cache/cargo/save").json(&requests[6]),
            server.post("/api/v1/cache/cargo/save").json(&requests[7]),
            server.post("/api/v1/cache/cargo/save").json(&requests[8]),
            server.post("/api/v1/cache/cargo/save").json(&requests[9]),
        );

        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        Ok(())
    }
}
