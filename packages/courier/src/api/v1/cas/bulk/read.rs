use aerosol::axum::Dep;
use async_tar::{Builder, Header};
use axum::{
    Json,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use clients::{ContentType, NETWORK_BUFFER_SIZE, courier::v1::cas::CasBulkReadRequest};
use futures::AsyncWriteExt;
use tokio_util::{
    compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    io::ReaderStream,
};
use tracing::{error, info};

use crate::storage::Disk;

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
/// key and contains blob content. The Accept header determines the format:
/// - `application/x-zstd-tar`: Each tar entry contains pre-compressed blob data
/// - Any other value: Each tar entry contains uncompressed blob data
///
/// The response sets `Content-Type`:
/// - `application/x-zstd-tar` indicates the CAS blobs are compressed
/// - `application/x-tar` indicates the CAS blobs are uncompressed
///
/// Note: The tar archive itself is always uncompressed. The Accept header only
/// indicates whether the individual blobs inside the tar should be compressed.
/// Missing keys are silently skipped.
///
/// ## Streaming
///
/// The tar archive is streamed directly to the client without buffering the
/// entire archive in memory. Each blob is read from disk and written to the
/// tar stream as it's processed.
#[tracing::instrument(skip(req))]
pub async fn handle(
    Dep(cas): Dep<Disk>,
    headers: HeaderMap,
    Json(req): Json<CasBulkReadRequest>,
) -> BulkReadResponse {
    info!(keys = req.keys.len(), "cas.bulk.read.start");

    let want_compressed = headers
        .get(ContentType::ACCEPT)
        .is_some_and(|accept| accept == ContentType::TarZstd);

    if want_compressed {
        handle_compressed(cas, req).await
    } else {
        handle_plain(cas, req).await
    }
}

#[tracing::instrument]
async fn handle_compressed(cas: Disk, req: CasBulkReadRequest) -> BulkReadResponse {
    info!("cas.bulk.read.compressed");
    let (reader, writer) = piper::pipe(NETWORK_BUFFER_SIZE);
    tokio::spawn(async move {
        let mut builder = Builder::new(writer);
        for key in req.keys {
            let reader = match cas.read_compressed(&key).await {
                Ok(reader) => reader,
                Err(error) => {
                    error!(%key, ?error, "cas.bulk.read.compressed.error");
                    continue;
                }
            };

            let bytes = match cas.size_compressed(&key).await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    error!(%key, error = "No compressed size for blob", "cas.bulk.read.size_compressed.error");
                    continue;
                }
                Err(error) => {
                    error!(%key, ?error, "cas.bulk.read.size_compressed.error");
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

    let stream = ReaderStream::with_capacity(reader.compat(), NETWORK_BUFFER_SIZE);
    let body = Body::from_stream(stream);
    BulkReadResponse::Success(body, ContentType::TarZstd)
}

#[tracing::instrument]
async fn handle_plain(cas: Disk, req: CasBulkReadRequest) -> BulkReadResponse {
    info!("cas.bulk.read.uncompressed");
    let (reader, writer) = piper::pipe(NETWORK_BUFFER_SIZE);
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

    let stream = ReaderStream::with_capacity(reader.compat(), NETWORK_BUFFER_SIZE);
    let body = Body::from_stream(stream);
    BulkReadResponse::Success(body, ContentType::Tar)
}

#[derive(Debug)]
pub enum BulkReadResponse {
    Success(Body, ContentType),
}

impl IntoResponse for BulkReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            BulkReadResponse::Success(body, ct) => {
                (StatusCode::OK, [(ContentType::HEADER, ct.value())], body).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use async_tar::Archive;
    use clients::{
        ContentType,
        courier::v1::{Key, cas::CasBulkReadRequest},
    };
    use color_eyre::{Result, eyre::Context};
    use futures::{StreamExt, io::Cursor};
    use maplit::btreemap;
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;
    use std::collections::BTreeMap;
    use tap::Pipe;
    use tokio_util::compat::FuturesAsyncReadCompatExt;

    use crate::api::test_helpers::write_cas;

    #[track_caller]
    fn decompress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::decompress(data.as_ref(), 10 * 1024 * 1024).expect("decompress")
    }

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

        let request = CasBulkReadRequest::builder()
            .keys([&key1, &key2, &key3])
            .build();

        let response = server.post("/api/v1/cas/bulk/read").json(&request).await;

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
        let missing_key =
            Key::from_hex("0000000000000000000000000000000000000000000000000000000000000000")?;
        let request = CasBulkReadRequest::builder()
            .keys([&key, &missing_key])
            .build();

        let response = server.post("/api/v1/cas/bulk/read").json(&request).await;

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

        let request = CasBulkReadRequest::default();

        let response = server.post("/api/v1/cas/bulk/read").json(&request).await;
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

        // Request with invalid keys should fail at deserialization.
        // We use raw JSON here since we can't construct invalid keys with the typed
        // builder.
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

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn bulk_read_compressed(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let content1 = b"first blob content";
        let content2 = b"second blob content";
        let content3 = b"third blob content";

        let key1 = write_cas(&server, content1).await?;
        let key2 = write_cas(&server, content2).await?;
        let key3 = write_cas(&server, content3).await?;

        let request = CasBulkReadRequest::builder()
            .keys([&key1, &key2, &key3])
            .build();

        let response = server
            .post("/api/v1/cas/bulk/read")
            .add_header(ContentType::ACCEPT, ContentType::TarZstd.value())
            .json(&request)
            .await;

        response.assert_status_ok();
        let content_type = response.header(ContentType::HEADER);
        pretty_assert_eq!(content_type, ContentType::TarZstd.value().to_str().unwrap());

        let tar_data = response.as_bytes();

        let cursor = Cursor::new(tar_data.to_vec());
        let archive = Archive::new(cursor);
        let mut entries = archive.entries()?;

        let mut found = BTreeMap::new();
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();

            let mut compressed = Vec::new();
            tokio::io::copy(&mut entry.compat(), &mut compressed).await?;

            let decompressed = decompress(&compressed);
            found.insert(path, decompressed);
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
    async fn bulk_read_uncompressed_explicit(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let content1 = b"first blob content";
        let content2 = b"second blob content";

        let key1 = write_cas(&server, content1).await?;
        let key2 = write_cas(&server, content2).await?;

        let request = CasBulkReadRequest::builder().keys([&key1, &key2]).build();

        let response = server
            .post("/api/v1/cas/bulk/read")
            .add_header(ContentType::ACCEPT, ContentType::Tar.value())
            .json(&request)
            .await;

        response.assert_status_ok();
        let content_type = response.header(ContentType::HEADER);
        pretty_assert_eq!(content_type, ContentType::Tar.value().to_str().unwrap());

        let tar_data = response.as_bytes();

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
        };

        pretty_assert_eq!(found, expected);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn bulk_read_compressed_missing_keys(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let content = b"existing blob";
        let key = write_cas(&server, content).await?;

        let missing_key =
            Key::from_hex("0000000000000000000000000000000000000000000000000000000000000000")?;
        let request = CasBulkReadRequest::builder()
            .keys([&key, &missing_key])
            .build();

        let response = server
            .post("/api/v1/cas/bulk/read")
            .add_header(ContentType::ACCEPT, ContentType::TarZstd.value())
            .json(&request)
            .await;

        response.assert_status_ok();
        let content_type = response.header(ContentType::HEADER);
        pretty_assert_eq!(content_type, ContentType::TarZstd.value().to_str().unwrap());

        let tar_data = response.as_bytes();

        let cursor = Cursor::new(tar_data.to_vec());
        let archive = Archive::new(cursor);
        let mut entries = archive.entries()?;

        let mut found = BTreeMap::new();
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();

            let mut compressed = Vec::new();
            tokio::io::copy(&mut entry.compat(), &mut compressed).await?;

            let decompressed = decompress(&compressed);
            found.insert(path, decompressed);
        }

        let expected = btreemap! {
            key.to_string() => content.to_vec(),
        };

        pretty_assert_eq!(found, expected);
        Ok(())
    }
}
