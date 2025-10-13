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
