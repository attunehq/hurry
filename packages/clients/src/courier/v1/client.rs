//! HTTP client for the Courier v1 API.

use std::{collections::HashMap, sync::Arc};

use async_compression::{
    Level,
    tokio::bufread::{ZstdDecoder, ZstdEncoder},
};
use async_tar::Archive;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, eyre},
};
use derive_more::{Debug, Display};
use futures::{AsyncWriteExt, Stream, StreamExt, TryStreamExt};
use reqwest::{Response, StatusCode};
use tap::Pipe;
use tokio::io::{AsyncRead, BufReader};
use tokio_util::{
    compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    io::{ReaderStream, StreamReader},
};
use tracing::instrument;
use url::Url;

use super::{
    Key,
    cache::{
        CargoBulkRestoreRequest, CargoBulkRestoreResponse, CargoRestoreRequest,
        CargoRestoreResponse, CargoSaveRequest,
    },
    cas::{CasBulkReadRequest, CasBulkWriteResponse},
};
use crate::{
    ContentType, NETWORK_BUFFER_SIZE, Token,
    courier::v1::cache::{CargoRestoreRequest2, CargoRestoreResponse2, CargoSaveRequest2},
};

/// Maximum decompressed size for individual blob decompression (1GB).
///
/// This limit applies per blob, including within bulk operations (e.g., each
/// entry in `cas_bulk_read_bytes_stream`). It does not limit the total size of
/// all blobs in a bulk operation or tar archive, only the size of each
/// decompressed blob.
const MAX_DECOMPRESSED_SIZE: usize = 1024 * 1024 * 1024;

/// Client for the Courier API.
///
/// ## Cloning
///
/// This type is cheaply cloneable, and clones share the underlying HTTP
/// connection pool.
#[derive(Clone, Debug, Display)]
#[display("{base}")]
pub struct Client {
    #[debug("{:?}", base.as_str())]
    base: Arc<Url>,

    #[debug(skip)]
    http: reqwest::Client,

    token: Token,
}
impl Client {
    /// Create a new client with the given base URL and authentication token.
    pub fn new(base: Url, token: Token) -> Result<Self> {
        let http = reqwest::Client::builder()
            .gzip(true)
            .brotli(true)
            .build()
            .context("build http client")?;

        Ok(Self {
            base: Arc::new(base),
            http,
            token,
        })
    }

    /// Check that the service is reachable.
    #[instrument(skip(self))]
    pub async fn ping(&self) -> Result<()> {
        let url = self.base.join("api/v1/health")?;
        let response = self.http.get(url).send().await.context("request")?;
        match response.status() {
            StatusCode::OK => Ok(()),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Save cargo cache metadata.
    ///
    /// Important: this is temporarily stubbed using the local file system.
    #[instrument(skip(self))]
    pub async fn cargo_cache_save2(&self, body: CargoSaveRequest2) -> Result<()> {
        tokio::fs::create_dir_all("/tmp/courier-v2-stub")
            .await
            .context("create stub directory")?;

        for request in body {
            let path = format!("/tmp/courier-v2-stub/{}.json", request.key.stable_hash());
            let json = serde_json::to_vec_pretty(&request.unit).context("serialize unit")?;
            tokio::fs::write(&path, json)
                .await
                .with_context(|| format!("write unit to {path}"))?;
        }

        Ok(())
    }

    /// Restore cargo cache metadata.
    ///
    /// Important: this is temporarily stubbed using the local file system.
    #[instrument(skip(self))]
    pub async fn cargo_cache_restore2(
        &self,
        body: CargoRestoreRequest2,
    ) -> Result<Option<CargoRestoreResponse2>> {
        let mut results = HashMap::new();
        for key in body {
            let path = format!("/tmp/courier-v2-stub/{}.json", key.stable_hash());
            match tokio::fs::read(&path).await {
                Ok(json) => {
                    let unit = serde_json::from_slice(&json)
                        .with_context(|| format!("deserialize unit from {path}"))?;
                    results.insert(key.clone(), unit);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("read unit from {path}"));
                }
            }
        }

        if results.is_empty() {
            Ok(None)
        } else {
            CargoRestoreResponse2::from(results).pipe(Some).pipe(Ok)
        }
    }

    /// Save cargo cache metadata.
    #[instrument(skip(self))]
    #[deprecated = "Replaced by `cargo_cache_save2`"]
    pub async fn cargo_cache_save(&self, body: CargoSaveRequest) -> Result<()> {
        let url = self.base.join("api/v1/cache/cargo/save")?;
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .json(&body)
            .send()
            .await
            .context("send")?;

        match response.status() {
            StatusCode::CREATED => Ok(()),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Restore cargo cache metadata.
    #[instrument(skip(self))]
    #[deprecated = "Replaced by `cargo_cache_restore2`"]
    pub async fn cargo_cache_restore(
        &self,
        body: CargoRestoreRequest,
    ) -> Result<Option<CargoRestoreResponse>> {
        let url = self.base.join("api/v1/cache/cargo/restore")?;
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .json(&body)
            .send()
            .await
            .context("send")?;

        match response.status() {
            StatusCode::OK => {
                let data = response
                    .json::<CargoRestoreResponse>()
                    .await
                    .context("parse JSON response")?;
                Ok(Some(data))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Restore multiple cargo cache entries in bulk.
    ///
    /// Note: The server supports up to 100,000 requests in a single bulk
    /// operation. If you exceed this limit, the server will return a 400
    /// Bad Request error.
    #[instrument(skip(self, requests))]
    #[deprecated = "Replaced by `cargo_cache_restore2`"]
    pub async fn cargo_cache_restore_bulk(
        &self,
        requests: impl IntoIterator<Item = impl Into<CargoRestoreRequest>>,
    ) -> Result<CargoBulkRestoreResponse> {
        let url = self.base.join("api/v1/cache/cargo/bulk/restore")?;
        let requests = requests.into_iter().map(Into::into).collect::<Vec<_>>();
        let body = CargoBulkRestoreRequest::builder()
            .requests(requests)
            .build();

        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .json(&body)
            .send()
            .await
            .context("send")?;

        match response.status() {
            StatusCode::OK => {
                let data = response
                    .json::<CargoBulkRestoreResponse>()
                    .await
                    .context("parse JSON response")?;
                Ok(data)
            }
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"))
            }
        }
    }

    /// Check if a CAS object exists.
    #[instrument(skip(self))]
    pub async fn cas_exists(&self, key: &Key) -> Result<bool> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self
            .http
            .head(url)
            .bearer_auth(self.token.expose())
            .send()
            .await
            .context("send")?;
        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Read a CAS object.
    #[instrument(skip(self))]
    pub async fn cas_read(&self, key: &Key) -> Result<Option<impl AsyncRead + Unpin>> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .send()
            .await
            .context("send")?;
        match response.status() {
            StatusCode::OK => response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .pipe(StreamReader::new)
                .pipe(BufReader::new)
                .pipe(ZstdDecoder::new)
                .pipe(Some)
                .pipe(Ok),
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Write a CAS object.
    #[instrument(skip(self, content))]
    pub async fn cas_write(
        &self,
        key: &Key,
        content: impl AsyncRead + Unpin + Send + 'static,
    ) -> Result<()> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let content = BufReader::new(content);
        let encoder = ZstdEncoder::with_quality(content, Level::Default);
        let stream = ReaderStream::with_capacity(encoder, NETWORK_BUFFER_SIZE);
        let body = reqwest::Body::wrap_stream(stream);

        let response = self
            .http
            .put(url)
            .bearer_auth(self.token.expose())
            .header(ContentType::HEADER, ContentType::BytesZstd.value())
            .body(body)
            .send()
            .await
            .context("send")?;
        match response.status() {
            StatusCode::CREATED => Ok(()),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Write a CAS object from bytes.
    #[instrument(name = "Client::cas_write_bytes", skip(body), fields(body = body.len()))]
    pub async fn cas_write_bytes(&self, key: &Key, body: Vec<u8>) -> Result<()> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let compressed = zstd::bulk::compress(&body, 0).context("compress body")?;
        let response = self
            .http
            .put(url)
            .bearer_auth(self.token.expose())
            .header(ContentType::HEADER, ContentType::BytesZstd.value())
            .body(compressed)
            .send()
            .await
            .context("send")?;
        match response.status() {
            StatusCode::CREATED => Ok(()),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"));
            }
        }
    }

    /// Read a CAS object into a byte vector.
    pub async fn cas_read_bytes(&self, key: &Key) -> Result<Option<Vec<u8>>> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .header(ContentType::ACCEPT, ContentType::BytesZstd.value())
            .send()
            .await
            .context("send")?;
        match response.status() {
            StatusCode::OK => {
                let compressed = response.bytes().await.context("read body")?;
                let decompressed = zstd::bulk::decompress(&compressed, MAX_DECOMPRESSED_SIZE)
                    .context("decompress body")?;
                Ok(Some(decompressed))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let url = response.url().to_string();
                let request_id = request_id(&response);
                let body = response.text().await.unwrap_or_default();
                Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
                    .with_section(|| request_id.header("Request ID:"))
            }
        }
    }

    /// Write multiple CAS objects from a tar archive.
    #[instrument(name = "Client::cas_write_bulk", skip(entries))]
    pub async fn cas_write_bulk(
        &self,
        mut entries: impl Stream<Item = (Key, Vec<u8>)> + Unpin + Send + 'static,
    ) -> Result<CasBulkWriteResponse> {
        let url = self.base.join("api/v1/cas/bulk/write")?;
        let (reader, writer) = piper::pipe(NETWORK_BUFFER_SIZE);
        let writer = tokio::task::spawn(async move {
            let mut tar = async_tar::Builder::new(writer);
            while let Some((key, content)) = entries.next().await {
                let compressed = zstd::bulk::compress(&content, 0)
                    .with_context(|| format!("compress entry: {key}"))?;
                let mut header = async_tar::Header::new_gnu();
                header.set_size(compressed.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, key.to_hex(), compressed.as_slice())
                    .await
                    .with_context(|| format!("add entry: {key}"))?;
            }

            let mut writer = tar.into_inner().await.context("finalize tarball")?;
            writer.close().await.context("close writer")
        });

        let stream = ReaderStream::with_capacity(reader.compat(), NETWORK_BUFFER_SIZE);
        let body = reqwest::Body::wrap_stream(stream);
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .header(ContentType::HEADER, ContentType::TarZstd.value())
            .body(body)
            .send()
            .await
            .context("send")?;
        writer
            .await
            .context("join archive task")?
            .context("write archive")?;

        let status = response.status();
        if status.is_success() {
            response
                .json::<CasBulkWriteResponse>()
                .await
                .context("parse")
        } else {
            let url = response.url().to_string();
            let request_id = request_id(&response);
            let body = response.text().await.unwrap_or_default();
            Err(eyre!("unexpected status code: {status}"))
                .with_section(|| url.header("Url:"))
                .with_section(|| body.header("Body:"))
                .with_section(|| request_id.header("Request ID:"))
        }
    }

    /// Read multiple CAS objects as tar archive bytes.
    #[instrument(name = "Client::cas_read_bulk", skip(keys))]
    pub async fn cas_read_bulk(
        &self,
        keys: impl IntoIterator<Item = impl Into<Key>>,
    ) -> Result<impl Stream<Item = Result<(Key, Vec<u8>)>> + Unpin> {
        let url = self.base.join("api/v1/cas/bulk/read")?;
        let request = CasBulkReadRequest::builder().keys(keys).build();
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .header(ContentType::ACCEPT, ContentType::TarZstd.value())
            .json(&request)
            .send()
            .await
            .context("send")?;

        let archive = response
            .bytes_stream()
            .map_err(std::io::Error::other)
            .pipe(StreamReader::new)
            .pipe(|r| Archive::new(r.compat()));

        let (tx, rx) = flume::bounded::<Result<(Key, Vec<u8>)>>(0);
        tokio::task::spawn(async move {
            let mut entries = match archive.entries().context("read entries") {
                Ok(entries) => entries,
                Err(err) => {
                    return tx
                        .send_async(Err(err))
                        .await
                        .expect("invariant: sender cannot be closed");
                }
            };
            let mut download = async || -> Result<()> {
                while let Some(entry) = entries.next().await {
                    let entry = entry.context("read entry")?;
                    let path = entry.path().context("read path")?;
                    let key = Key::from_hex(path.to_string_lossy())
                        .with_context(|| format!("parse entry name {path:?}"))?;

                    let mut compressed = Vec::new();
                    tokio::io::copy(&mut entry.compat(), &mut compressed)
                        .await
                        .context("read compressed content")?;

                    let decompressed = zstd::bulk::decompress(&compressed, MAX_DECOMPRESSED_SIZE)
                        .with_context(|| format!("decompress entry: {key}"))?;

                    tx.send_async(Ok((key, decompressed)))
                        .await
                        .expect("invariant: sender cannot be closed");
                }
                Result::<()>::Ok(())
            };
            while let Err(err) = download().await {
                tx.send_async(Err(err))
                    .await
                    .expect("invariant: sender cannot be closed");
            }
        });

        rx.into_stream().pipe(Ok)
    }

    /// Reset all cache data: delete all database records and CAS blobs.
    #[instrument(skip(self))]
    pub async fn cache_reset(&self) -> Result<()> {
        let url = self.base.join("api/v1/cache/cargo/reset")?;
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token.expose())
            .send()
            .await
            .context("send")?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            status => {
                let url = response.url().to_string();
                let body = response.text().await.unwrap_or_default();
                Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
            }
        }
    }
}

/// Extract the request ID from a response header.
fn request_id(response: &Response) -> String {
    response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| String::from("<not set>"))
}
