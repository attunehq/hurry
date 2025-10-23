use aerosol::axum::Dep;
use axum::{
    body::Body,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use clients::{ContentType, NETWORK_BUFFER_SIZE};
use color_eyre::{Result, eyre::Report};
use tokio_util::io::ReaderStream;
use tracing::{error, info};

use crate::storage::{Disk, Key};

/// Read the content from the CAS for the given key.
///
/// This handler implements the GET endpoint for retrieving blob content. It
/// streams the content from disk.
///
/// ## Response format
///
/// The Accept header in the request determines the format:
/// - `application/octet-stream+zstd`: The body is compressed with `zstd`.
/// - Any other value: The body is uncompressed.
///
/// The response sets `Content-Type`:
/// - `application/octet-stream+zstd`: The body is compressed with `zstd`.
/// - `application/octet-stream`: The body is uncompressed.
#[tracing::instrument]
pub async fn handle(
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
    headers: HeaderMap,
) -> CasReadResponse {
    // Check Accept header to determine if client wants compressed response
    let want_compressed = headers
        .get(ContentType::ACCEPT)
        .is_some_and(|accept| accept == ContentType::BytesZstd);

    let payload = if want_compressed {
        handle_compressed(cas, key)
            .await
            .map(|body| (body, ContentType::BytesZstd))
    } else {
        handle_plain(cas, key)
            .await
            .map(|body| (body, ContentType::Bytes))
    };

    match payload {
        Ok((body, ct)) => CasReadResponse::Found(body, ct),
        Err(err) => {
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

#[tracing::instrument]
async fn handle_compressed(cas: Disk, key: Key) -> Result<Body> {
    info!("cas.read.compressed");
    cas.read_compressed(&key)
        .await
        .map(|s| ReaderStream::with_capacity(s, NETWORK_BUFFER_SIZE))
        .map(Body::from_stream)
}

#[tracing::instrument]
async fn handle_plain(cas: Disk, key: Key) -> Result<Body> {
    info!("cas.read.uncompressed");
    cas.read(&key)
        .await
        .map(|s| ReaderStream::with_capacity(s, NETWORK_BUFFER_SIZE))
        .map(Body::from_stream)
}

#[derive(Debug)]
pub enum CasReadResponse {
    Found(Body, ContentType),
    NotFound,
    Error(Report),
}

impl IntoResponse for CasReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasReadResponse::Found(body, ct) => {
                (StatusCode::OK, [(ContentType::HEADER, ct.value())], body).into_response()
            }
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
    use clients::ContentType;
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, write_cas};

    #[track_caller]
    fn decompress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::decompress(data.as_ref(), 10 * 1024 * 1024).expect("decompress")
    }

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

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn read_compressed(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"test content for compression";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let key = write_cas(&server, CONTENT).await?;

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .await;

        response.assert_status_ok();
        let content_type = response.header(ContentType::HEADER);
        pretty_assert_eq!(
            content_type,
            ContentType::BytesZstd.value().to_str().unwrap()
        );

        let compressed_body = response.as_bytes();
        let decompressed = decompress(compressed_body);
        pretty_assert_eq!(decompressed.as_slice(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn read_uncompressed_explicit(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"test content without compression";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let key = write_cas(&server, CONTENT).await?;

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header(ContentType::ACCEPT, ContentType::Bytes.value())
            .await;

        response.assert_status_ok();
        let content_type = response.header(ContentType::HEADER);
        pretty_assert_eq!(content_type, ContentType::Bytes.value().to_str().unwrap());

        let body = response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn read_compressed_nonexistent_key(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, nonexistent_key) = test_blob(b"never written");

        let response = server
            .get(&format!("/api/v1/cas/{nonexistent_key}"))
            .add_header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }
}
