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
    match db.check_cas_access(&auth, &key).await {
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
