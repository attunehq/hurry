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
