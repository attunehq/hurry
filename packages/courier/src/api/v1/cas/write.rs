use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use futures::TryStreamExt;
use tokio_util::io::StreamReader;
use tracing::{error, info};

use crate::storage::{Disk, Key};

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
#[tracing::instrument(skip(body))]
pub async fn handle(Dep(cas): Dep<Disk>, Path(key): Path<Key>, body: Body) -> CasWriteResponse {
    let stream = body.into_data_stream();
    let stream = stream.map_err(std::io::Error::other);
    let reader = StreamReader::new(stream);

    // Note: [`Disk::write`] validates that the content hashes to the provided
    // key. If the hash doesn't match, the write fails and we return an error.
    match cas.write(&key, reader).await {
        Ok(()) => {
            info!("cas.write.success");
            CasWriteResponse::Created
        }
        Err(err) => {
            error!(error = ?err, "cas.write.error");
            CasWriteResponse::Error(err)
        }
    }
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
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;

    use crate::api::test_helpers::{test_blob, write_cas};

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn basic_write_flow(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"hello world";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .bytes(Bytes::from_static(CONTENT))
            .await;

        response.assert_status(StatusCode::CREATED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn idempotent_writes(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"idempotent test";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        // First write
        let response1 = server
            .put(&format!("/api/v1/cas/{key}"))
            .bytes(Bytes::from_static(CONTENT))
            .await;
        response1.assert_status(StatusCode::CREATED);

        // Second write
        let response2 = server
            .put(&format!("/api/v1/cas/{key}"))
            .bytes(Bytes::from_static(CONTENT))
            .await;
        response2.assert_status(StatusCode::CREATED);

        // Content should still be readable
        let read_response = server.get(&format!("/api/v1/cas/{key}")).await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn invalid_key_hash(pool: PgPool) -> Result<()> {
        const ACTUAL_CONTENT: &[u8] = b"actual content";
        const WRONG_CONTENT: &[u8] = b"different content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, wrong_key) = test_blob(WRONG_CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{wrong_key}"))
            .bytes(Bytes::from_static(ACTUAL_CONTENT))
            .await;

        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn large_blob_write(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let content = vec![0xAB; 1024 * 1024]; // 1MB blob
        let key = write_cas(&server, &content).await?;

        // Verify it can be read back
        let response = server.get(&format!("/api/v1/cas/{key}")).await;
        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.len(), content.len());
        pretty_assert_eq!(body.as_ref(), content.as_slice());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn concurrent_writes_same_blob(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"concurrent write test content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        // Execute 10 concurrent writes of the same content
        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .bytes(Bytes::from_static(CONTENT)),
        );

        // All writes should succeed (idempotent)
        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        // Verify content is correct and uncorrupted
        let read_response = server.get(&format!("/api/v1/cas/{key}")).await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT, "content should be uncorrupted");

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    async fn concurrent_writes_different_blobs(pool: PgPool) -> Result<()> {
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

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
                .bytes(Bytes::copy_from_slice(&blobs[0].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[1].0))
                .bytes(Bytes::copy_from_slice(&blobs[1].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[2].0))
                .bytes(Bytes::copy_from_slice(&blobs[2].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[3].0))
                .bytes(Bytes::copy_from_slice(&blobs[3].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[4].0))
                .bytes(Bytes::copy_from_slice(&blobs[4].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[5].0))
                .bytes(Bytes::copy_from_slice(&blobs[5].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[6].0))
                .bytes(Bytes::copy_from_slice(&blobs[6].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[7].0))
                .bytes(Bytes::copy_from_slice(&blobs[7].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[8].0))
                .bytes(Bytes::copy_from_slice(&blobs[8].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[9].0))
                .bytes(Bytes::copy_from_slice(&blobs[9].1)),
        );

        // All writes should succeed
        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        // Verify all blobs can be read back with correct content
        for (key, expected_content) in blobs {
            let read_response = server.get(&format!("/api/v1/cas/{key}")).await;
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
}
