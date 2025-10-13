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
