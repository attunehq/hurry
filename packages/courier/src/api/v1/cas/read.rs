use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use tokio_util::io::ReaderStream;
use tracing::{error, info};

use crate::storage::{Disk, Key};

/// Read the content from the CAS for the given key.
///
/// This handler implements the GET endpoint for retrieving blob content. It
/// streams the content from disk (decompressing on the fly).
#[tracing::instrument]
pub async fn handle(Dep(cas): Dep<Disk>, Path(key): Path<Key>) -> CasReadResponse {
    match cas.read(&key).await {
        Ok(reader) => {
            info!("cas.read.success");
            let stream = ReaderStream::new(reader);
            CasReadResponse::Found(Body::from_stream(stream))
        }
        Err(err) => {
            // Check if the error is a "file not found" error by examining the error chain
            let is_not_found = err.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
            });

            if is_not_found {
                info!("cas.read.not_found");
                CasReadResponse::NotFound
            } else {
                error!(error = ?err, "cas.read.error");
                CasReadResponse::Error(err)
            }
        }
    }
}

#[derive(Debug)]
pub enum CasReadResponse {
    Found(Body),
    NotFound,
    Error(Report),
}

impl IntoResponse for CasReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasReadResponse::Found(body) => (StatusCode::OK, body).into_response(),
            CasReadResponse::NotFound => StatusCode::NOT_FOUND.into_response(),
            CasReadResponse::Error(error) => {
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
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, write_cas};

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn read_after_write(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"read test content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let key = write_cas(&server, CONTENT).await?;

        let response = server.get(&format!("/api/v1/cas/{key}")).await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn read_nonexistent_key(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, nonexistent_key) = test_blob(b"never written");

        let response = server.get(&format!("/api/v1/cas/{nonexistent_key}")).await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn read_large_blob(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let content = vec![0xFF; 5 * 1024 * 1024]; // 5MB blob
        let key = write_cas(&server, &content).await?;

        let response = server.get(&format!("/api/v1/cas/{key}")).await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.len(), content.len());
        pretty_assert_eq!(body.as_ref(), content.as_slice());

        Ok(())
    }
}
