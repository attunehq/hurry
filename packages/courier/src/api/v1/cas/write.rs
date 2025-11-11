use aerosol::axum::Dep;
use axum::{
    body::Body,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use clients::ContentType;
use color_eyre::{Result, eyre::Report};
use futures::{StreamExt, TryStreamExt};
use tap::Pipe;
use tokio_util::io::StreamReader;
use tracing::{error, info};

use crate::{
    auth::AuthenticatedToken,
    db::Postgres,
    storage::{Disk, Key},
};

/// Write the content to the CAS for the given key.
///
/// This handler implements the PUT endpoint for storing blob content. It
/// streams the request body to disk (compressing with zstd) and validates the
/// hash matches the provided key.
///
/// ## Idempotency
///
/// The CAS is idempotent: if a file already exists, it is not written again.
/// This is safe because the key is computed from the content of the file, so if
/// the file already exists it must have the same content.
///
/// ## Atomic writes
///
/// The CAS uses write-then-rename to ensure that writes are atomic. If a file
/// already exists, it is not written again. This is safe because the key is
/// computed from the content of the file, so if the file already exists it must
/// have the same content.
///
/// ## Key validation
///
/// While clients provide the key to the request, the CAS validates the key when
/// the content is written to ensure that the key provided by the user and the
/// key computed from the content actually match.
///
/// If they do not, this request is rejected and the write operation is aborted.
/// Making clients provide the key is due to two reasons:
/// 1. It reduces the chance that the client provides the wrong value.
/// 2. It allows this service to colocate the temporary file with the ultimate
///    destination for the content, which makes implementation simpler if we
///    move to multiple mounted disks for subsets of the CAS.
///
/// ## Compression
///
/// The `Content-Type` header communicates the format:
/// - `application/octet-stream+zstd`: The body is compressed with `zstd`.
/// - Any other value: The body is uncompressed.
///
/// Pre-compressed content is validated to ensure it decompresses correctly and
/// hashes to the expected key.
#[tracing::instrument(skip(auth, body))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
    headers: HeaderMap,
    body: Body,
) -> CasWriteResponse {
    // Check if the key already exists before consuming the body
    // If it exists, we still need to consume the entire body; if we return early
    // instead then clients see a "connection reset by peer" error.
    let exists = match cas.exists(&key).await {
        Ok(exists) => exists,
        Err(err) => {
            error!(error = ?err, "cas.write.exists.error");
            return CasWriteResponse::Error(err);
        }
    };

    if exists {
        // Consume and discard the body to avoid client connection errors. But even if
        // we for some reason fail to drain, report it as a success anyway.
        body.into_data_stream().for_each(|_| async {}).await;

        // Grant access even though it already exists (idempotent, in case org didn't
        // have access)
        match db.grant_cas_access(auth.org_id, &key).await {
            Ok(granted) => {
                info!(?granted, "cas.write.exists");
                return CasWriteResponse::Created;
            }
            Err(err) => {
                error!(error = ?err, "cas.write.grant_access_error");
                return CasWriteResponse::Error(err);
            }
        }
    }

    // Check Content-Type header to determine if content is pre-compressed
    let is_compressed = headers
        .get(ContentType::HEADER)
        .is_some_and(|v| v == ContentType::BytesZstd);

    let result = if is_compressed {
        handle_compressed(cas, key.clone(), body).await
    } else {
        handle_plain(cas, key.clone(), body).await
    };

    match result {
        Ok(()) => {
            // Grant org access to the CAS key after successful write
            match db.grant_cas_access(auth.org_id, &key).await {
                Ok(granted) => {
                    info!(?granted, "cas.write.success");
                    CasWriteResponse::Created
                }
                Err(err) => {
                    error!(error = ?err, "cas.write.grant_access_error");
                    CasWriteResponse::Error(err)
                }
            }
        }
        Err(err) => {
            error!(error = ?err, "cas.write.error");
            CasWriteResponse::Error(err)
        }
    }
}

#[tracing::instrument(skip(body))]
async fn handle_compressed(cas: Disk, key: Key, body: Body) -> Result<()> {
    info!("cas.write.compressed");
    let stream = body
        .into_data_stream()
        .map_err(std::io::Error::other)
        .pipe(StreamReader::new);
    cas.write_compressed(&key, stream).await
}

#[tracing::instrument(skip(body))]
async fn handle_plain(cas: Disk, key: Key, body: Body) -> Result<()> {
    info!("cas.write.uncompressed");
    let stream = body
        .into_data_stream()
        .map_err(std::io::Error::other)
        .pipe(StreamReader::new);
    cas.write(&key, stream).await
}

#[derive(Debug)]
pub enum CasWriteResponse {
    Created,
    Error(Report),
}

impl IntoResponse for CasWriteResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            CasWriteResponse::Created => StatusCode::CREATED.into_response(),
            CasWriteResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Bytes;
    use axum::http::StatusCode;
    use clients::ContentType;
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, test_server, write_cas};

    #[track_caller]
    fn compress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::compress(data.as_ref(), 0).expect("compress")
    }

    #[track_caller]
    fn decompress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::decompress(data.as_ref(), 10 * 1024 * 1024).expect("decompress")
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn basic_write_flow(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"hello world";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);
        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::from_static(CONTENT))
            .await;

        response.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn idempotent_writes(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"idempotent test";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        // First write
        let response1 = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::from_static(CONTENT))
            .await;
        response1.assert_status(StatusCode::CREATED);

        // Second write
        let response2 = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::from_static(CONTENT))
            .await;
        response2.assert_status(StatusCode::CREATED);

        // Content should still be readable

        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn idempotent_write_large_blob(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let content = vec![0xCD; 2 * 1024 * 1024]; // 2MB blob
        let (_, key) = test_blob(&content);

        // First write
        let response1 = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&content))
            .await;
        response1.assert_status(StatusCode::CREATED);

        // Second write with the same content
        // This tests that the server fully consumes the body even when the file
        // already exists, avoiding "connection reset by peer" errors
        let response2 = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&content))
            .await;
        response2.assert_status(StatusCode::CREATED);

        // Verify content is still readable and correct
        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.len(), content.len());
        pretty_assert_eq!(body.as_ref(), content.as_slice());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn invalid_key_hash(pool: PgPool) -> Result<()> {
        const ACTUAL_CONTENT: &[u8] = b"actual content";
        const WRONG_CONTENT: &[u8] = b"different content";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, wrong_key) = test_blob(WRONG_CONTENT);
        let response = server
            .put(&format!("/api/v1/cas/{wrong_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::from_static(ACTUAL_CONTENT))
            .await;

        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn large_blob_write(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let content = vec![0xAB; 1024 * 1024]; // 1MB blob
        let key = write_cas(&server, &content, auth.token_alice().expose()).await?;

        // Verify it can be read back
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.len(), content.len());
        pretty_assert_eq!(body.as_ref(), content.as_slice());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn concurrent_writes_same_blob(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"concurrent write test content";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        // Execute 10 concurrent writes of the same content
        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::from_static(CONTENT)),
        );

        // All writes should succeed (idempotent)
        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        // Verify content is correct and uncorrupted
        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT, "content should be uncorrupted");

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn concurrent_writes_different_blobs(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Create 10 different blobs
        let blobs = (0..10)
            .map(|i| {
                let content = format!("concurrent blob {i}").into_bytes();
                let (_, key) = test_blob(&content);
                (key, content)
            })
            .collect::<Vec<_>>();

        // Write all blobs concurrently
        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .put(&format!("/api/v1/cas/{}", blobs[0].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[0].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[1].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[1].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[2].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[2].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[3].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[3].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[4].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[4].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[5].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[5].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[6].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[6].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[7].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[7].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[8].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[8].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[9].0))
                .authorization_bearer(auth.token_alice().expose())
                .bytes(Bytes::copy_from_slice(&blobs[9].1)),
        );

        // All writes should succeed
        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        // Verify all blobs can be read back with correct content
        for (key, expected_content) in blobs {
            let read_response = server
                .get(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .await;
            read_response.assert_status_ok();
            let body = read_response.as_bytes();
            pretty_assert_eq!(
                body.as_ref(),
                expected_content.as_slice(),
                "blob content should match for key {key}"
            );
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_compressed(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"test content for compression";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);
        let compressed = compress(CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .content_type(ContentType::BytesZstd.to_str())
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&compressed))
            .await;

        response.assert_status(StatusCode::CREATED);

        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_compressed_idempotent(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"idempotent compressed test";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);
        let compressed = compress(CONTENT);

        let response1 = server
            .put(&format!("/api/v1/cas/{key}"))
            .content_type(ContentType::BytesZstd.to_str())
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&compressed))
            .await;
        response1.assert_status(StatusCode::CREATED);

        let response2 = server
            .put(&format!("/api/v1/cas/{key}"))
            .content_type(ContentType::BytesZstd.to_str())
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&compressed))
            .await;
        response2.assert_status(StatusCode::CREATED);

        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_compressed_invalid_hash(pool: PgPool) -> Result<()> {
        const ACTUAL_CONTENT: &[u8] = b"actual content";
        const WRONG_CONTENT: &[u8] = b"different content";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, wrong_key) = test_blob(WRONG_CONTENT);
        let compressed = compress(ACTUAL_CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{wrong_key}"))
            .content_type(ContentType::BytesZstd.to_str())
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&compressed))
            .await;

        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_compressed_roundtrip(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"roundtrip test content";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(CONTENT);
        let compressed = compress(CONTENT);

        let write_response = server
            .put(&format!("/api/v1/cas/{key}"))
            .content_type(ContentType::BytesZstd.to_str())
            .authorization_bearer(auth.token_alice().expose())
            .bytes(Bytes::copy_from_slice(&compressed))
            .await;
        write_response.assert_status(StatusCode::CREATED);

        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .authorization_bearer(auth.token_alice().expose())
            .await;

        read_response.assert_status_ok();
        let content_type = read_response.header(ContentType::HEADER);
        pretty_assert_eq!(
            content_type,
            ContentType::BytesZstd.value().to_str().unwrap()
        );

        let compressed_body = read_response.as_bytes();
        let decompressed = decompress(compressed_body);
        pretty_assert_eq!(decompressed.as_slice(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_missing_auth_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, key) = test_blob(b"test content");

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .bytes(content.into())
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_invalid_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, key) = test_blob(b"test content");

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer("invalid-token-that-does-not-exist")
            .bytes(content.into())
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn write_revoked_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, key) = test_blob(b"test content");

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice_revoked().expose())
            .bytes(content.into())
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }
}
