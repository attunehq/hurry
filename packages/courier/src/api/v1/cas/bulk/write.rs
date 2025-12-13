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
        handle_compressed(&auth, db, cas, body).await
    } else {
        handle_plain(&auth, db, cas, body).await
    }
}

#[tracing::instrument(skip(auth, body))]
async fn handle_compressed(
    auth: &AuthenticatedToken,
    db: Postgres,
    cas: Disk,
    body: Body,
) -> BulkWriteResponse {
    info!("cas.bulk.write.compressed");
    process_archive(auth, db, cas, body, true).await
}

#[tracing::instrument(skip(auth, body))]
async fn handle_plain(
    auth: &AuthenticatedToken,
    db: Postgres,
    cas: Disk,
    body: Body,
) -> BulkWriteResponse {
    info!("cas.bulk.write.uncompressed");
    process_archive(auth, db, cas, body, false).await
}

async fn process_archive(
    auth: &AuthenticatedToken,
    db: Postgres,
    cas: Disk,
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
            match db.grant_cas_access(auth, &key).await {
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
            Ok(()) => match db.grant_cas_access(auth, &key).await {
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
