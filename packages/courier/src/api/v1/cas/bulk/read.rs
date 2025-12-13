use std::collections::HashSet;

use aerosol::axum::Dep;
use async_tar::{Builder, Header};
use axum::{
    Json,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use clients::{
    ContentType, NETWORK_BUFFER_SIZE,
    courier::v1::{Key, cas::CasBulkReadRequest},
};
use color_eyre::Report;
use futures::AsyncWriteExt;
use tokio_util::{
    compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    io::ReaderStream,
};
use tracing::{Instrument, error, info};

use crate::{auth::AuthenticatedToken, db::Postgres, storage::Disk};

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
#[tracing::instrument(skip(auth, req))]
pub async fn handle(
    auth: AuthenticatedToken,
    Dep(db): Dep<Postgres>,
    Dep(cas): Dep<Disk>,
    headers: HeaderMap,
    Json(req): Json<CasBulkReadRequest>,
) -> BulkReadResponse {
    info!(keys = req.keys.len(), "cas.bulk.read.start");

    let accessible_keys = match db.check_cas_access_bulk(&auth, &req.keys).await {
        Ok(keys) => keys,
        Err(error) => {
            error!(?error, "cas.bulk.read.access_check_bulk.error");
            return BulkReadResponse::Error(error);
        }
    };

    let want_compressed = headers
        .get(ContentType::ACCEPT)
        .is_some_and(|accept| accept == ContentType::TarZstd);

    if want_compressed {
        handle_compressed(cas, accessible_keys, req).await
    } else {
        handle_plain(cas, accessible_keys, req).await
    }
}

#[tracing::instrument(skip(accessible_keys))]
async fn handle_compressed(
    cas: Disk,
    accessible_keys: HashSet<Key>,
    req: CasBulkReadRequest,
) -> BulkReadResponse {
    info!("cas.bulk.read.compressed");

    let (reader, writer) = piper::pipe(NETWORK_BUFFER_SIZE);
    let span = tracing::info_span!("cas_bulk_read_compressed_worker");
    tokio::spawn(
        async move {
            let mut builder = Builder::new(writer);
            for key in req.keys {
                // Check if org has access to this key
                if !accessible_keys.contains(&key) {
                    error!(%key, "cas.bulk.read.no_access");
                    continue;
                }

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
        }
        .instrument(span),
    );

    let stream = ReaderStream::with_capacity(reader.compat(), NETWORK_BUFFER_SIZE);
    let body = Body::from_stream(stream);
    BulkReadResponse::Success(body, ContentType::TarZstd)
}

#[tracing::instrument(skip(accessible_keys))]
async fn handle_plain(
    cas: Disk,
    accessible_keys: HashSet<Key>,
    req: CasBulkReadRequest,
) -> BulkReadResponse {
    info!("cas.bulk.read.uncompressed");

    let (reader, writer) = piper::pipe(NETWORK_BUFFER_SIZE);
    let span = tracing::info_span!("cas_bulk_read_plain_worker");
    tokio::spawn(
        async move {
            let mut builder = Builder::new(writer);
            for key in req.keys {
                // Check if org has access to this key
                if !accessible_keys.contains(&key) {
                    error!(%key, "cas.bulk.read.no_access");
                    continue;
                }

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
        }
        .instrument(span),
    );

    let stream = ReaderStream::with_capacity(reader.compat(), NETWORK_BUFFER_SIZE);
    let body = Body::from_stream(stream);
    BulkReadResponse::Success(body, ContentType::Tar)
}

#[derive(Debug)]
pub enum BulkReadResponse {
    Success(Body, ContentType),
    Error(Report),
}

impl IntoResponse for BulkReadResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            BulkReadResponse::Success(body, ct) => {
                (StatusCode::OK, [(ContentType::HEADER, ct.value())], body).into_response()
            }
            BulkReadResponse::Error(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{err:?}")).into_response()
            }
        }
    }
}
