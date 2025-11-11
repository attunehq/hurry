use std::collections::BTreeSet;

use aerosol::axum::Dep;
use async_tar::Archive;
use axum::{
    Json,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use clients::{
    ContentType,
    courier::v1::cas::{CasBulkWriteKeyError, CasBulkWriteResponse},
};
use color_eyre::{Report, eyre::Context};
use futures::StreamExt;
use tap::Pipe;
use tokio_util::{
    compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    io::StreamReader,
};
use tracing::{error, info};

use crate::{
    auth::AuthenticatedToken,
    db::Postgres,
    storage::{Disk, Key},
};

/// Responses for bulk write operation.
pub enum BulkWriteResponse {
    Success(CasBulkWriteResponse),
    PartialSuccess(CasBulkWriteResponse),
    InvalidRequest(Report),
    Error(Report),
}

/// Write multiple blobs to the CAS from a tar archive.
///
/// This handler implements the POST endpoint for bulk blob storage. It accepts
/// a tar archive where each entry is named with the hex-encoded key and
/// contains the blob content to store.
///
/// ## Request format
///
/// The request body should be a tar archive where each entry is named with the
/// hex-encoded key. The `Content-Type` header determines the format:
/// - `application/x-tar`: Each tar entry contains uncompressed blob data
/// - `application/x-zstd-tar`: Each tar entry contains pre-compressed blob data
///
/// Note: The tar archive itself is always uncompressed. The Content-Type only
/// indicates whether the individual blobs inside the tar are compressed.
///
/// ## Response format
///
/// ```json
/// {
///   "written": ["abc123...", "def456..."],
///   "skipped": ["ghi789..."],
///   "errors": [
///     {"key": "jkl012...", "error": "hash mismatch"}
///   ]
/// }
/// ```
///
/// ## Idempotency
///
/// Like single-item writes, bulk writes are idempotent. If a blob already
/// exists in storage, it's reported in the "skipped" array and not written
/// again.
///
/// ## Partial success
///
/// The bulk write operation uses a partial success model: if some blobs fail
/// to write, the operation continues processing remaining blobs and returns
/// a summary of successes, skips, and errors.
///
/// ## Validation
///
/// Each blob is validated during write to ensure its content hashes to the
/// provided key, just like single-item writes.
#[tracing::instrument(skip(auth, body))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    headers: HeaderMap,
    body: Body,
) -> BulkWriteResponse {
    info!("cas.bulk.write.start");

    // Check Content-Type to determine if entries are pre-compressed
    let entries_compressed = headers
        .get(ContentType::HEADER)
        .is_some_and(|v| v == ContentType::TarZstd);

    if entries_compressed {
        handle_compressed(db, cas, auth, body).await
    } else {
        handle_plain(db, cas, auth, body).await
    }
}

#[tracing::instrument(skip(body))]
async fn handle_compressed(
    db: Postgres,
    cas: Disk,
    auth: AuthenticatedToken,
    body: Body,
) -> BulkWriteResponse {
    info!("cas.bulk.write.compressed");
    process_archive(db, cas, auth, body, true).await
}

#[tracing::instrument(skip(body))]
async fn handle_plain(
    db: Postgres,
    cas: Disk,
    auth: AuthenticatedToken,
    body: Body,
) -> BulkWriteResponse {
    info!("cas.bulk.write.uncompressed");
    process_archive(db, cas, auth, body, false).await
}

async fn process_archive(
    db: Postgres,
    cas: Disk,
    auth: AuthenticatedToken,
    body: Body,
    entries_compressed: bool,
) -> BulkWriteResponse {
    let stream = body.into_data_stream();
    let stream = stream.map(|result| result.map_err(std::io::Error::other));
    let archive = StreamReader::new(stream).compat().pipe(Archive::new);
    let mut entries = match archive.entries().context("read archive entries") {
        Ok(entries) => entries,
        Err(error) => {
            error!(?error, "cas.bulk.write.request.read");
            return BulkWriteResponse::Error(error);
        }
    };

    let mut written = BTreeSet::new();
    let mut skipped = BTreeSet::new();
    let mut errors = BTreeSet::new();
    while let Some(entry) = entries.next().await {
        let entry = match entry.context("read archive entry") {
            Ok(entry) => entry,
            Err(error) => {
                error!(?error, "cas.bulk.write.entry.read");
                return BulkWriteResponse::InvalidRequest(error);
            }
        };

        let path = match entry.path().context("read path for entry") {
            Ok(path) => path,
            Err(error) => {
                error!(?error, "cas.bulk.write.entry.path");
                return BulkWriteResponse::InvalidRequest(error);
            }
        };

        let path = path.to_string_lossy();
        let key = match Key::from_hex(&path) {
            Ok(key) => key,
            Err(error) => {
                error!(?error, ?path, "cas.bulk.write.entry.path.parse");
                return BulkWriteResponse::InvalidRequest(error);
            }
        };

        // We still need to grant access, even if the CAS item exists.
        if let Ok(true) = cas.exists(&key).await {
            match db.grant_cas_access(auth.org_id, &key).await {
                Ok(granted) => {
                    if granted {
                        // Org didn't have access, to them this was "written"
                        info!(%key, "cas.bulk.write.exists.granted");
                        written.insert(key);
                    } else {
                        // Org already had access, so this was "skipped"
                        info!(%key, "cas.bulk.write.skipped");
                        skipped.insert(key);
                    }
                }
                Err(error) => {
                    error!(%key, ?error, "cas.bulk.write.grant_access.error");
                    errors.insert(
                        CasBulkWriteKeyError::builder()
                            .key(key)
                            .error(format!("blob exists but failed to grant access: {error:?}"))
                            .build(),
                    );
                }
            }
            continue;
        }

        let result = if entries_compressed {
            cas.write_compressed(&key, entry.compat()).await
        } else {
            cas.write(&key, entry.compat()).await
        };

        match result {
            Ok(()) => match db.grant_cas_access(auth.org_id, &key).await {
                Ok(granted) => {
                    info!(%key, ?granted, "cas.bulk.write.success");
                    written.insert(key);
                }
                Err(error) => {
                    error!(%key, ?error, "cas.bulk.write.grant_access.error");
                    errors.insert(
                        CasBulkWriteKeyError::builder()
                            .key(key)
                            .error(format!(
                                "write succeeded but failed to grant access: {error:?}"
                            ))
                            .build(),
                    );
                }
            },
            Err(error) => {
                error!(%key, ?error, "cas.bulk.write.error");
                errors.insert(
                    CasBulkWriteKeyError::builder()
                        .key(key)
                        .error(format!("{error:?}"))
                        .build(),
                );
            }
        }
    }

    info!(
        written = written.len(),
        skipped = skipped.len(),
        errors = errors.len(),
        "cas.bulk.write.complete"
    );

    let partial = !errors.is_empty();
    let body = CasBulkWriteResponse::builder()
        .written(written)
        .skipped(skipped)
        .errors(errors)
        .build();
    if partial {
        BulkWriteResponse::PartialSuccess(body)
    } else {
        BulkWriteResponse::Success(body)
    }
}

impl IntoResponse for BulkWriteResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            BulkWriteResponse::Success(body) => (StatusCode::CREATED, Json(body)).into_response(),
            BulkWriteResponse::PartialSuccess(body) => {
                (StatusCode::ACCEPTED, Json(body)).into_response()
            }
            BulkWriteResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
            BulkWriteResponse::InvalidRequest(error) => {
                (StatusCode::BAD_REQUEST, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use async_tar::{Builder, Header};
    use axum::http::StatusCode;
    use clients::{ContentType, courier::v1::cas::CasBulkWriteResponse};
    use color_eyre::{Result, eyre::Context};
    use futures::{StreamExt, io::Cursor};
    use maplit::btreeset;
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use sqlx::PgPool;
    use tokio_util::compat::FuturesAsyncReadCompatExt;

    use crate::api::test_helpers::{test_blob, test_server};

    #[track_caller]
    fn compress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::compress(data.as_ref(), 0).expect("compress")
    }

    #[track_caller]
    fn decompress(data: impl AsRef<[u8]>) -> Vec<u8> {
        zstd::bulk::decompress(data.as_ref(), 10 * 1024 * 1024).expect("decompress")
    }

    /// Helper to create a tar archive with the given blobs
    async fn create_tar(blobs: Vec<(impl AsRef<str>, impl AsRef<[u8]>)>) -> Result<Vec<u8>> {
        let cursor = Cursor::new(Vec::new());
        let mut builder = Builder::new(cursor);

        for (key, content) in blobs {
            let (key, content) = (key.as_ref(), content.as_ref());
            let mut header = Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();

            let cursor = Cursor::new(content);
            builder.append_data(&mut header, key, cursor).await?;
        }

        let cursor = builder.into_inner().await?;
        Ok(cursor.into_inner())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_multiple_blobs(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content1, key1) = test_blob(b"first blob content");
        let (content2, key2) = test_blob(b"second blob content");
        let (content3, key3) = test_blob(b"third blob content");

        let tar_data = create_tar(vec![
            (key1.to_hex(), content1.to_vec()),
            (key2.to_hex(), content2.to_vec()),
            (key3.to_hex(), content3.to_vec()),
        ])
        .await?;

        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar_data.into())
            .await;

        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        let expected = CasBulkWriteResponse::builder()
            .written([&key1, &key2, &key3])
            .build();
        pretty_assert_eq!(body, expected);

        for (key, expected) in [(key1, content1), (key2, content2), (key3, content3)] {
            let read_response = server
                .get(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .await;
            read_response.assert_status_ok();
            let body = read_response.as_bytes();
            pretty_assert_eq!(body.as_ref(), expected.as_slice());
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_idempotent(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, key) = test_blob(b"idempotent blob");
        let tar_data = create_tar(vec![(key.to_hex(), content)]).await?;

        let response1 = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar_data.clone().into())
            .await;

        response1.assert_status_success();
        let body1 = response1.json::<CasBulkWriteResponse>();

        let expected1 = CasBulkWriteResponse::builder().written([&key]).build();
        pretty_assert_eq!(body1, expected1);

        let response2 = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar_data.into())
            .await;

        response2.assert_status_success();
        let body2 = response2.json::<CasBulkWriteResponse>();

        // Second write by same org should be reported as "skipped" (org already had
        // access)
        let expected2 = CasBulkWriteResponse::builder().skipped([&key]).build();
        pretty_assert_eq!(body2, expected2);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_invalid_hash(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, _) = test_blob(b"actual content");
        let (_, key) = test_blob(b"different content");

        let tar = create_tar(vec![(key.to_hex(), content)]).await?;
        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar.into())
            .await;

        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        // For errors, we can't predict the exact error message, so check structure
        pretty_assert_eq!(body.written, BTreeSet::new());
        pretty_assert_eq!(body.skipped, BTreeSet::new());
        pretty_assert_eq!(body.errors.len(), 1);

        let error = body.errors.iter().next().unwrap();
        pretty_assert_eq!(&error.key, &key);
        assert!(error.error.contains("hash mismatch"));

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_invalid_filename(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let tar = create_tar(vec![("not-a-valid-hex-key", b"test content")]).await?;
        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar.into())
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_partial_success(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (valid_content, valid_key) = test_blob(b"valid content");
        let (wrong_content, _) = test_blob(b"actual content");
        let (_, wrong_key) = test_blob(b"different content");

        let tar_data = create_tar(vec![
            (valid_key.to_hex(), &valid_content),
            (wrong_key.to_hex(), &wrong_content),
        ])
        .await?;

        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar_data.into())
            .await;
        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        // Check the written and skipped parts match exactly
        pretty_assert_eq!(body.written, btreeset! { valid_key.clone() });
        pretty_assert_eq!(body.skipped, BTreeSet::new());

        // For errors, we can't predict the exact error message, so check structure
        pretty_assert_eq!(body.errors.len(), 1);
        let error = body.errors.iter().next().unwrap();
        pretty_assert_eq!(&error.key, &wrong_key);

        let response = server
            .get(&format!("/api/v1/cas/{valid_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        response.assert_status_ok();
        pretty_assert_eq!(response.as_bytes().as_ref(), &valid_content);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_empty_tar(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let tar = create_tar(Vec::<(&str, &[u8])>::new()).await?;
        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar.into())
            .await;

        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        let expected = CasBulkWriteResponse::default();
        pretty_assert_eq!(body, expected);

        Ok(())
    }

    /// Helper to create a tar archive with compressed blobs
    async fn create_tar_compressed(
        blobs: Vec<(impl AsRef<str>, impl AsRef<[u8]>)>,
    ) -> Result<Vec<u8>> {
        let cursor = Cursor::new(Vec::new());
        let mut builder = Builder::new(cursor);

        for (key, content) in blobs {
            let (key, content) = (key.as_ref(), content.as_ref());
            let compressed = compress(content);
            let mut header = Header::new_gnu();
            header.set_size(compressed.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();

            let cursor = Cursor::new(compressed);
            builder.append_data(&mut header, key, cursor).await?;
        }

        let cursor = builder.into_inner().await?;
        Ok(cursor.into_inner())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_compressed(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content1, key1) = test_blob(b"first blob content");
        let (content2, key2) = test_blob(b"second blob content");
        let (content3, key3) = test_blob(b"third blob content");

        let tar_data = create_tar_compressed(vec![
            (key1.to_hex(), content1.to_vec()),
            (key2.to_hex(), content2.to_vec()),
            (key3.to_hex(), content3.to_vec()),
        ])
        .await?;

        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type(ContentType::TarZstd.to_str())
            .bytes(tar_data.into())
            .await;

        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        let expected = CasBulkWriteResponse::builder()
            .written([&key1, &key2, &key3])
            .build();
        pretty_assert_eq!(body, expected);

        for (key, expected) in [(key1, content1), (key2, content2), (key3, content3)] {
            let read_response = server
                .get(&format!("/api/v1/cas/{key}"))
                .authorization_bearer(auth.token_alice().expose())
                .await;
            read_response.assert_status_ok();
            let body = read_response.as_bytes();
            pretty_assert_eq!(body.as_ref(), expected.as_slice());
        }

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_compressed_idempotent(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, key) = test_blob(b"idempotent compressed blob");
        let tar_data = create_tar_compressed(vec![(key.to_hex(), content.clone())]).await?;

        let response1 = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type(ContentType::TarZstd.to_str())
            .bytes(tar_data.clone().into())
            .await;

        response1.assert_status_success();
        let body1 = response1.json::<CasBulkWriteResponse>();

        let expected1 = CasBulkWriteResponse::builder().written([&key]).build();
        pretty_assert_eq!(body1, expected1);

        let response2 = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type(ContentType::TarZstd.to_str())
            .bytes(tar_data.into())
            .await;

        response2.assert_status_success();
        let body2 = response2.json::<CasBulkWriteResponse>();

        let expected2 = CasBulkWriteResponse::builder().skipped([&key]).build();
        pretty_assert_eq!(body2, expected2);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_compressed_invalid_hash(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, _) = test_blob(b"actual content");
        let (_, key) = test_blob(b"different content");

        let tar = create_tar_compressed(vec![(key.to_hex(), content)]).await?;
        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type(ContentType::TarZstd.to_str())
            .bytes(tar.into())
            .await;

        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        pretty_assert_eq!(body.written, BTreeSet::new());
        pretty_assert_eq!(body.skipped, BTreeSet::new());
        pretty_assert_eq!(body.errors.len(), 1);

        let error = body.errors.iter().next().unwrap();
        pretty_assert_eq!(&error.key, &key);
        assert!(error.error.contains("hash mismatch"));

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_compressed_partial_success(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (valid_content, valid_key) = test_blob(b"valid content");
        let (wrong_content, _) = test_blob(b"actual content");
        let (_, wrong_key) = test_blob(b"different content");

        let tar_data = create_tar_compressed(vec![
            (valid_key.to_hex(), &valid_content),
            (wrong_key.to_hex(), &wrong_content),
        ])
        .await?;

        let response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type(ContentType::TarZstd.to_str())
            .bytes(tar_data.into())
            .await;
        response.assert_status_success();
        let body = response.json::<CasBulkWriteResponse>();

        pretty_assert_eq!(body.written, btreeset! { valid_key.clone() });
        pretty_assert_eq!(body.skipped, BTreeSet::new());

        pretty_assert_eq!(body.errors.len(), 1);
        let error = body.errors.iter().next().unwrap();
        pretty_assert_eq!(&error.key, &wrong_key);

        let response = server
            .get(&format!("/api/v1/cas/{valid_key}"))
            .authorization_bearer(auth.token_alice().expose())
            .await;
        response.assert_status_ok();
        pretty_assert_eq!(response.as_bytes().as_ref(), &valid_content);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_compressed_roundtrip(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content1, key1) = test_blob(b"first blob");
        let (content2, key2) = test_blob(b"second blob");

        let tar_data = create_tar_compressed(vec![
            (key1.to_hex(), content1.to_vec()),
            (key2.to_hex(), content2.to_vec()),
        ])
        .await?;

        let write_response = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type(ContentType::TarZstd.to_str())
            .bytes(tar_data.into())
            .await;

        write_response.assert_status_success();

        let request = clients::courier::v1::cas::CasBulkReadRequest::builder()
            .keys([&key1, &key2])
            .build();

        let read_response = server
            .post("/api/v1/cas/bulk/read")
            .authorization_bearer(auth.token_alice().expose())
            .add_header(clients::ContentType::ACCEPT, ContentType::TarZstd.value())
            .json(&request)
            .await;

        read_response.assert_status_ok();
        let content_type = read_response.header(clients::ContentType::HEADER);
        pretty_assert_eq!(content_type, ContentType::TarZstd.value().to_str().unwrap());

        let tar_data = read_response.as_bytes();
        let cursor = Cursor::new(tar_data.to_vec());
        let archive = async_tar::Archive::new(cursor);
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

        let expected = maplit::btreemap! {
            key1.to_string() => content1.to_vec(),
            key2.to_string() => content2.to_vec(),
        };

        pretty_assert_eq!(found, expected);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::db::Postgres::MIGRATOR")]
    #[test_log::test]
    async fn bulk_write_grants_access_when_blob_exists(pool: PgPool) -> Result<()> {
        let (server, auth, _tmp) = test_server(pool).await.context("create test server")?;

        let (content, key) = test_blob(b"shared blob content");

        // Alice uploads the blob first
        let tar_data_alice = create_tar(vec![(key.to_hex(), content.clone())]).await?;
        let response_alice = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_alice().expose())
            .content_type("application/x-tar")
            .bytes(tar_data_alice.into())
            .await;

        response_alice.assert_status_success();
        let body_alice = response_alice.json::<CasBulkWriteResponse>();
        pretty_assert_eq!(body_alice.written, btreeset! { key.clone() });

        // Charlie (different org) should not be able to read the blob yet
        let check_response = server
            .method(axum::http::Method::HEAD, &format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_charlie().expose())
            .await;
        check_response.assert_status(StatusCode::NOT_FOUND);

        // Charlie uploads the same blob (blob exists on disk, but Charlie doesn't have
        // access)
        let tar_data_charlie = create_tar(vec![(key.to_hex(), content.clone())]).await?;
        let response_charlie = server
            .post("/api/v1/cas/bulk/write")
            .authorization_bearer(auth.token_charlie().expose())
            .content_type("application/x-tar")
            .bytes(tar_data_charlie.into())
            .await;

        response_charlie.assert_status_success();
        let body_charlie = response_charlie.json::<CasBulkWriteResponse>();

        // Should be reported as "written" because Charlie's org didn't have access
        // before
        pretty_assert_eq!(body_charlie.written, btreeset! { key.clone() });
        pretty_assert_eq!(body_charlie.skipped, BTreeSet::new());
        pretty_assert_eq!(body_charlie.errors, BTreeSet::new());

        // Now Charlie should be able to read the blob
        let read_response = server
            .get(&format!("/api/v1/cas/{key}"))
            .authorization_bearer(auth.token_charlie().expose())
            .await;
        read_response.assert_status_ok();
        pretty_assert_eq!(read_response.as_bytes().as_ref(), content.as_slice());

        Ok(())
    }
}
