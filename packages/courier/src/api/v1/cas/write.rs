use aerosol::axum::Dep;
use axum::{body::Body, extract::Path, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use futures::TryStreamExt;
use tokio_util::io::StreamReader;
use tracing::{error, info, warn};

use crate::{
    auth::{KeySets, StatelessToken},
    db::Postgres,
    storage::{Disk, Key},
};

/// Write the content to the CAS for the given key.
///
/// This handler implements the PUT endpoint for storing blob content. It streams the
/// request body to disk (compressing with zstd), validates the hash matches the
/// provided key, grants database access to the organization, and asynchronously
/// records access frequency for cache warming.
///
/// ## Security
///
/// All accounts have visibility into all keys that any account in the organization
/// has ever written. This is intentional, because we expect accounts to be used
/// by developers on their local machines as well as in CI or other environments
/// like docker builds.
///
/// Even if another organization has written content with the same key, this
/// content is not visible to the current organization unless they have also
/// written it.
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
pub async fn handle(
    token: StatelessToken,
    Dep(keysets): Dep<KeySets>,
    Dep(cas): Dep<Disk>,
    Dep(db): Dep<Postgres>,
    Path(key): Path<Key>,
    body: Body,
) -> CasWriteResponse {
    let stream = body.into_data_stream();
    let stream = stream.map_err(std::io::Error::other);
    let reader = StreamReader::new(stream);

    // Note: [`Disk::write`] validates that the content hashes to the provided
    // key. If the hash doesn't match, the write fails and we return an error
    // without granting database access.
    match cas.write(&key, reader).await {
        Ok(()) => {
            // Grant org access to key in database.
            //
            // We write to disk first, then grant database access. This ordering
            // means that if the database grant fails, we'll have an orphaned
            // blob on disk that no org can access. This is acceptable because:
            // 1. We can't transact across disk and database
            // 2. Writes are idempotent, a retry will succeed
            // 3. Storage is cheaper than blocking writes on database operations
            // 4. Orphaned blobs are a tolerable edge case vs. high write
            //    latency
            // 5. We will likely add a cleanup job for orphaned temp blobs in
            //    the future (reference comments around temp files in
            //    [`Disk::write`]) and we can just clean these up at the same
            //    time.
            if let Err(err) = db.grant_org_cas_key(token.org_id, &key).await {
                error!(?err, account = ?token.account_id, org = ?token.org_id, "grant org access to cas key");
                return CasWriteResponse::Error(err);
            }

            keysets.organization(token.org_id).insert(key.clone());

            // We record access frequency asynchronously to avoid blocking
            // the overall request, since access frequency is a "nice to
            // have" feature while latency is a "must have" feature.
            let account_id = token.account_id;
            tokio::spawn(async move {
                if let Err(err) = db.record_cas_key_access(account_id, &key).await {
                    warn!(error = ?err, "cas.write.record_access_failed");
                }
            });

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

    use crate::api::test_helpers::{mint_token, test_blob, write_cas};

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn basic_write_flow(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const CONTENT: &[u8] = b"hello world";
        let (server, _tmp) = crate::api::test_server(pool.clone())
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, key) = test_blob(CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .bytes(Bytes::from_static(CONTENT))
            .await;

        response.assert_status(StatusCode::CREATED);

        // Verify org has access to key in database
        let access: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM cas_access ca
                JOIN cas_key ck ON ca.cas_key_id = ck.id
                WHERE ca.org_id = $1 AND ck.content = $2
            )",
        )
        .bind(1i64)
        .bind(key.as_bytes())
        .fetch_one(&pool)
        .await?;

        pretty_assert_eq!(access, true, "org should have access to key");

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn write_grants_access_to_org(pool: PgPool) -> Result<()> {
        const ACCOUNT1_TOKEN: &str = "test-api-key-account1-org1";
        const ACCOUNT2_TOKEN: &str = "test-api-key-account2-org1";
        const CONTENT: &[u8] = b"shared content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        // Account1 writes content
        let account1_token = mint_token(&server, ACCOUNT1_TOKEN, 1).await?;
        let key = write_cas(&server, &account1_token, CONTENT).await?;

        // Account2 (same org) can read it
        let account2_token = mint_token(&server, ACCOUNT2_TOKEN, 1).await?;
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &account2_token)
            .await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(
            body.as_ref(),
            CONTENT,
            "account2 should read content written by account1"
        );

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn idempotent_writes(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const CONTENT: &[u8] = b"idempotent test";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, key) = test_blob(CONTENT);

        // First write
        let response1 = server
            .put(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .bytes(Bytes::from_static(CONTENT))
            .await;
        response1.assert_status(StatusCode::CREATED);

        // Second write
        let response2 = server
            .put(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .bytes(Bytes::from_static(CONTENT))
            .await;
        response2.assert_status(StatusCode::CREATED);

        // Content should still be readable
        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn invalid_key_hash(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const ACTUAL_CONTENT: &[u8] = b"actual content";
        const WRONG_CONTENT: &[u8] = b"different content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, wrong_key) = test_blob(WRONG_CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{wrong_key}"))
            .add_header("Authorization", &token)
            .bytes(Bytes::from_static(ACTUAL_CONTENT))
            .await;

        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn large_blob_write(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let content = vec![0xAB; 1024 * 1024]; // 1MB blob
        let key = write_cas(&server, &token, &content).await?;

        // Verify it can be read back
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;
        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.len(), content.len());
        pretty_assert_eq!(body.as_ref(), content.as_slice());

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn write_without_auth(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"test content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .bytes(Bytes::from_static(CONTENT))
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn write_with_raw_token_instead_of_stateless(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const CONTENT: &[u8] = b"test content";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let (_, key) = test_blob(CONTENT);

        // Try to use raw token directly (should fail)
        let response = server
            .put(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", format!("Bearer {TOKEN}"))
            .bytes(Bytes::from_static(CONTENT))
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn concurrent_writes_same_blob(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        const CONTENT: &[u8] = b"concurrent write test content";
        let (server, _tmp) = crate::api::test_server(pool.clone())
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;
        let (_, key) = test_blob(CONTENT);

        // Execute 10 concurrent writes of the same content
        let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
            server
                .put(&format!("/api/v1/cas/{key}"))
                .add_header("Authorization", &token)
                .bytes(Bytes::from_static(CONTENT)),
        );

        // All writes should succeed (idempotent)
        for response in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
            response.assert_status(StatusCode::CREATED);
        }

        // Verify content is correct and uncorrupted
        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header("Authorization", &token)
            .await;
        read_response.assert_status_ok();
        let body = read_response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT, "content should be uncorrupted");

        // Verify database shows org has access (only once, not 10 times)
        let access_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cas_access ca
            JOIN cas_key ck ON ca.cas_key_id = ck.id
            WHERE ca.org_id = $1 AND ck.content = $2",
        )
        .bind(1i64)
        .bind(key.as_bytes())
        .fetch_one(&pool)
        .await?;

        pretty_assert_eq!(access_count, 1, "should have exactly one access grant");

        Ok(())
    }

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn concurrent_writes_different_blobs(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-api-key-account1-org1";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let token = mint_token(&server, TOKEN, 1).await?;

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
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[0].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[1].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[1].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[2].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[2].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[3].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[3].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[4].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[4].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[5].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[5].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[6].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[6].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[7].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[7].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[8].0))
                .add_header("Authorization", &token)
                .bytes(Bytes::copy_from_slice(&blobs[8].1)),
            server
                .put(&format!("/api/v1/cas/{}", blobs[9].0))
                .add_header("Authorization", &token)
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
                .add_header("Authorization", &token)
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
}
