use aerosol::axum::Dep;
use async_tar::{Builder, Header};
use axum::{Json, body::Body, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use futures::AsyncWriteExt;
use serde::Deserialize;
use tokio_util::{
    compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    io::ReaderStream,
};
use tracing::{error, info};

use crate::storage::{Disk, Key};

/// Request body for bulk read operation.
#[derive(Debug, Deserialize)]
pub struct BulkReadRequest {
    pub keys: Vec<Key>,
}

/// Read multiple blobs from the CAS and return them as a tar archive.
///
/// This handler implements the POST endpoint for bulk blob retrieval. It
/// accepts a JSON body with an array of keys and returns a tar archive
/// containing all requested blobs that exist in storage.
///
/// ## Request format
///
/// ```json
/// {
///   "keys": ["abc123...", "def456..."]
/// }
/// ```
///
/// ## Response format
///
/// The response is a tar archive where each entry is named with the hex-encoded
/// key and contains the uncompressed blob content. Missing keys are silently
/// skipped.
///
/// ## Streaming
///
/// The tar archive is streamed directly to the client without buffering the
/// entire archive in memory. Each blob is read from disk and written to the
/// tar stream as it's processed.
///
/// ## Compression
///
/// The HTTP layer handles transparent compression (gzip/zstd) via the
/// CompressionLayer middleware, so the tar archive itself contains
/// uncompressed data.
#[tracing::instrument(skip(req))]
pub async fn handle(Dep(cas): Dep<Disk>, Json(req): Json<BulkReadRequest>) -> BulkReadResponse {
    info!(keys = req.keys.len(), "cas.bulk.read.start");

    let (reader, writer) = piper::pipe(64 * 1024);
    tokio::spawn(async move {
        let mut builder = Builder::new(writer);
        for key in req.keys {
            let reader = match cas.read(&key).await {
                Ok(reader) => reader,
                Err(error) => {
                    error!(%key, ?error, "cas.bulk.read.error");
                    continue;
                }
            };

            let bytes = match cas.size(&key).await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    error!(%key, error = "No size for blob", "cas.bulk.read.size.error");
                    continue;
                }
                Err(error) => {
                    error!(%key, ?error, "cas.bulk.read.size.error");
                    continue;
                }
            };
            let header = {
                let name = key.to_hex();
                let mut header = Header::new_gnu();
                if let Err(error) = header.set_path(&name) {
                    error!(%key, ?error, ?name, "cas.bulk.read.header.set_path.error");
                    continue;
                }
                header.set_size(bytes);
                header.set_mode(0o644);
                header.set_cksum();
                header
            };

            match builder.append(&header, reader.compat()).await {
                Ok(_) => info!(%key, bytes, "cas.bulk.read.append.success"),
                Err(error) => error!(%key, ?error, "cas.bulk.read.append.error"),
            }
        }

        // Finalize the tar archive and close the pipe.
        match builder.into_inner().await {
            Ok(mut writer) => match writer.close().await {
                Ok(_) => info!("cas.bulk.read.finalize.success"),
                Err(error) => error!(?error, "cas.bulk.read.finalize.error"),
            },
            Err(error) => error!(?error, "cas.bulk.read.finalize_error"),
        }
    });

    // Convert the pipe into a stream; this is streamed out to the client as it
    // is written from the background task.
    let stream = ReaderStream::with_capacity(reader.compat(), 1024 * 1024);
    let body = Body::from_stream(stream);
    BulkReadResponse::Success(body)
}

#[derive(Debug)]
pub enum BulkReadResponse {
    Success(Body),
    #[allow(dead_code)]
    Error(Report),
}

impl IntoResponse for BulkReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            BulkReadResponse::Success(body) => (
                StatusCode::OK,
                [("content-type", "application/x-tar")],
                body,
            )
                .into_response(),
            BulkReadResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use async_tar::Archive;
    use color_eyre::{Result, eyre::Context};
    use futures::{StreamExt, io::Cursor};
    use maplit::btreemap;
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;
    use std::collections::BTreeMap;
    use tap::Pipe;
    use tokio_util::compat::FuturesAsyncReadCompatExt;

    use crate::api::test_helpers::write_cas;

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn bulk_read_multiple_blobs(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Write three blobs
        let content1 = b"first blob content";
        let content2 = b"second blob content";
        let content3 = b"third blob content";

        let key1 = write_cas(&server, content1).await?;
        let key2 = write_cas(&server, content2).await?;
        let key3 = write_cas(&server, content3).await?;

        // Request bulk read
        let request_body = serde_json::json!({
            "keys": [key1.to_string(), key2.to_string(), key3.to_string()]
        });

        let response = server
            .post("/api/v1/cas/bulk/read")
            .json(&request_body)
            .await;

        response.assert_status_ok();
        let tar_data = response.as_bytes();

        // Parse the tar archive
        let cursor = Cursor::new(tar_data.to_vec());
        let archive = Archive::new(cursor);
        let mut entries = archive.entries()?;

        let mut found = BTreeMap::new();
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();

            let mut content = Vec::new();
            tokio::io::copy(&mut entry.compat(), &mut content).await?;
            found.insert(path, content);
        }

        let expected = btreemap! {
            key1.to_string() => content1.to_vec(),
            key2.to_string() => content2.to_vec(),
            key3.to_string() => content3.to_vec(),
        };

        pretty_assert_eq!(found, expected);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn bulk_read_missing_keys(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Write one blob
        let content = b"existing blob";
        let key = write_cas(&server, content).await?;

        // Request with one valid and one missing key
        let missing_key = "0000000000000000000000000000000000000000000000000000000000000000";
        let request_body = serde_json::json!({
            "keys": [key.to_string(), missing_key]
        });

        let response = server
            .post("/api/v1/cas/bulk/read")
            .json(&request_body)
            .await;

        response.assert_status_ok();
        let tar_data = response.as_bytes();

        // Parse the tar archive
        let cursor = Cursor::new(tar_data.to_vec());
        let archive = Archive::new(cursor);
        let mut entries = archive.entries()?;

        let mut found = BTreeMap::new();
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();

            let mut content = Vec::new();
            tokio::io::copy(&mut entry.compat(), &mut content).await?;
            found.insert(path, content);
        }

        let expected = btreemap! {
            key.to_string() => content.to_vec(),
        };

        pretty_assert_eq!(found, expected);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn bulk_read_empty_request(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let request_body = serde_json::json!({
            "keys": []
        });

        let response = server
            .post("/api/v1/cas/bulk/read")
            .json(&request_body)
            .await;
        response.assert_status_ok();

        let archive = response.as_bytes().pipe(Cursor::new).pipe(Archive::new);
        let mut entries = archive.entries()?;

        let mut found = BTreeMap::new();
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();

            let mut content = Vec::new();
            tokio::io::copy(&mut entry.compat(), &mut content).await?;
            found.insert(path, content);
        }

        let expected = BTreeMap::new();
        pretty_assert_eq!(found, expected);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn bulk_read_invalid_keys(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Request with invalid keys should fail at deserialization
        let request_body = serde_json::json!({
            "keys": ["not-a-hex-key", "also-invalid"]
        });

        let response = server
            .post("/api/v1/cas/bulk/read")
            .json(&request_body)
            .await;

        // Should return 422 Unprocessable Entity for invalid keys
        response.assert_status(axum::http::StatusCode::UNPROCESSABLE_ENTITY);

        Ok(())
    }
}
