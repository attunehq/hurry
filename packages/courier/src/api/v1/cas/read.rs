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

use crate::{
    auth::AuthenticatedToken,
    db::Postgres,
    storage::{Disk, Key},
};

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
#[tracing::instrument(skip(auth))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    Path(key): Path<Key>,
    headers: HeaderMap,
) -> CasReadResponse {
    // Check if org has access to this CAS key
    // Return NotFound (not Forbidden) to avoid leaking information about blob
    // existence
    match db.check_cas_access(auth.org_id, &key).await {
        Ok(true) => {}
        Ok(false) => {
            info!("cas.read.no_access");
            return CasReadResponse::NotFound;
        }
        Err(err) => {
            error!(error = ?err, "cas.read.access_check_error");
            return CasReadResponse::Error(err);
        }
    }

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

    use crate::api::test_helpers::{test_blob, test_server, write_cas};

    #[track_caller]
    fn decompress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::decompress(data.as_ref(), 10 * 1024 * 1024).expect("decompress")
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_after_write(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"read test content";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let key = write_cas(&server, CONTENT, auth.token_alice().expose()).await?;

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status_ok();
        let body = response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_nonexistent_key(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, nonexistent_key) = test_blob(b"never written");
        let response = server
            .get(&format!("/api/v1/cas/{nonexistent_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_large_blob(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let content = vec![0xFF; 5 * 1024 * 1024]; // 5MB blob
        let key = write_cas(&server, &content, auth.token_alice().expose()).await?;

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
    async fn read_compressed(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"test content for compression";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let key = write_cas(&server, CONTENT, auth.token_alice().expose()).await?;

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .authorization_bearer(auth.token_alice().expose())
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
    #[test_log::test]
    async fn read_uncompressed_explicit(pool: PgPool) -> Result<()> {
        const CONTENT: &[u8] = b"test content without compression";
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let key = write_cas(&server, CONTENT, auth.token_alice().expose()).await?;
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .add_header(ContentType::ACCEPT, ContentType::Bytes.value())
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status_ok();
        let content_type = response.header(ContentType::HEADER);
        pretty_assert_eq!(content_type, ContentType::Bytes.value().to_str().unwrap());

        let body = response.as_bytes();
        pretty_assert_eq!(body.as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_compressed_nonexistent_key(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, nonexistent_key) = test_blob(b"never written");
        let response = server
            .get(&format!("/api/v1/cas/{nonexistent_key}"))
            .add_header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_missing_auth_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"test content");

        let response = server.get(&format!("/api/v1/cas/{key}")).await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_invalid_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, _auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"test content");

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer("invalid-token-that-does-not-exist")
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn read_revoked_token_returns_401(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (_, key) = test_blob(b"test content");

        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice_revoked().expose())
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);

        Ok(())
    }

    // Multi-tenant isolation tests

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn org_cannot_read_other_orgs_blob(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        const CONTENT: &[u8] = b"org A private data";

        // Org A (Acme) writes a blob
        let key = write_cas(&server, CONTENT, auth.token_alice().expose()).await?;

        // Org B (Widget) tries to read it - should get 404 (blob appears non-existent)
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_charlie().expose())
            .await;

        response.assert_status(StatusCode::NOT_FOUND);

        // Org A can still read it
        let response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status_ok();
        pretty_assert_eq!(response.as_bytes().as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn same_content_different_orgs_separate_access(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        const CONTENT: &[u8] = b"shared content";
        let (_, key) = test_blob(CONTENT);

        // Org A writes the content
        let response_a = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .bytes(CONTENT.to_vec().into())
            .await;
        response_a.assert_status(StatusCode::CREATED);

        // Org B cannot read it yet (hasn't been granted access)
        let response_b_read = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_charlie().expose())
            .await;
        response_b_read.assert_status(StatusCode::NOT_FOUND);

        // Org B writes the same content (grants them access)
        let response_b = server
            .put(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_charlie().expose())
            .bytes(CONTENT.to_vec().into())
            .await;
        response_b.assert_status(StatusCode::CREATED);

        // Both orgs can read it
        let response_a = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        response_a.assert_status_ok();
        pretty_assert_eq!(response_a.as_bytes().as_ref(), CONTENT);

        let response_b = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_charlie().expose())
            .await;
        response_b.assert_status_ok();
        pretty_assert_eq!(response_b.as_bytes().as_ref(), CONTENT);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn same_org_users_can_access_each_others_blobs(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        // Alice writes a blob
        const ALICE_CONTENT: &[u8] = b"Alice's data";
        let alice_key = write_cas(&server, ALICE_CONTENT, auth.token_alice().expose()).await?;

        // Bob (same org) can read Alice's blob
        let response = server
            .get(&format!("/api/v1/cas/{alice_key}"))
            .authorization_bearer(auth.token_bob().expose())
            .await;

        response.assert_status_ok();
        pretty_assert_eq!(response.as_bytes().as_ref(), ALICE_CONTENT);

        // Bob writes a blob
        const BOB_CONTENT: &[u8] = b"Bob's data";
        let bob_key = write_cas(&server, BOB_CONTENT, auth.token_bob().expose()).await?;

        // Alice can read Bob's blob
        let response = server
            .get(&format!("/api/v1/cas/{bob_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;

        response.assert_status_ok();
        pretty_assert_eq!(response.as_bytes().as_ref(), BOB_CONTENT);

        Ok(())
    }
}
