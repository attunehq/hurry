use std::{collections::HashSet, sync::Arc};

use async_tar::Archive;
use bon::Builder;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, eyre},
};
use derive_more::{Debug, Deref, Display, From};
use futures::{AsyncWriteExt, Stream, StreamExt, TryStreamExt};
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use tap::Pipe;
use tokio::io::AsyncRead;
use tokio_util::{
    compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    io::{ReaderStream, StreamReader},
};
use tracing::instrument;
use url::Url;

use crate::{cargo::QualifiedPath, hash::Blake3};

/// Client for the Courier API.
///
/// ## Cloning
///
/// This type is cheaply cloneable, and clones share the underlying HTTP
/// connection pool.
#[derive(Clone, Debug, Display)]
#[display("{base}")]
pub struct Courier {
    #[debug("{:?}", base.as_str())]
    base: Arc<Url>,

    #[debug(skip)]
    http: reqwest::Client,
}

impl Courier {
    /// Create a new client with the given base URL.
    pub fn new(base: Url) -> Result<Self> {
        let http = reqwest::Client::builder()
            .gzip(true)
            .brotli(true)
            .build()
            .context("build http client")?;

        Ok(Self {
            base: Arc::new(base),
            http,
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

    /// Check if a CAS object exists.
    #[instrument(skip(self))]
    pub async fn cas_exists(&self, key: &Blake3) -> Result<bool> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self.http.head(url).send().await.context("send")?;
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
    pub async fn cas_read(&self, key: &Blake3) -> Result<Option<impl AsyncRead + Unpin>> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self.http.get(url).send().await.context("send")?;
        match response.status() {
            StatusCode::OK => response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .pipe(StreamReader::new)
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
        key: &Blake3,
        content: impl AsyncRead + Unpin + Send + 'static,
    ) -> Result<()> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let stream = ReaderStream::with_capacity(content, 1024 * 1024);
        let body = reqwest::Body::wrap_stream(stream);

        let response = self.http.put(url).body(body).send().await.context("send")?;
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

    /// Save cargo cache metadata.
    #[instrument(skip(self))]
    pub async fn cargo_cache_save(&self, body: CargoSaveRequest) -> Result<()> {
        let url = self.base.join("api/v1/cache/cargo/save")?;
        let response = self
            .http
            .post(url)
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
    pub async fn cargo_cache_restore(
        &self,
        body: CargoRestoreRequest,
    ) -> Result<Option<CargoRestoreResponse>> {
        let url = self.base.join("api/v1/cache/cargo/restore")?;
        let response = self
            .http
            .post(url)
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

    /// Write a CAS object from bytes.
    #[instrument(name = "Courier::cas_write_bytes", skip(body), fields(body = body.len()))]
    pub async fn cas_write_bytes(&self, key: &Blake3, body: Vec<u8>) -> Result<()> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self.http.put(url).body(body).send().await.context("send")?;
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
    pub async fn cas_read_bytes(&self, key: &Blake3) -> Result<Option<Vec<u8>>> {
        let url = self.base.join(&format!("api/v1/cas/{key}"))?;
        let response = self.http.get(url).send().await.context("send")?;
        match response.status() {
            StatusCode::OK => response
                .bytes()
                .await
                .context("read body")
                .map(|body| body.to_vec())
                .map(Some),
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
    #[instrument(name = "Courier::cas_write_bulk", skip(entries))]
    pub async fn cas_write_bulk(
        &self,
        mut entries: impl Stream<Item = (Blake3, Vec<u8>)> + Unpin + Send + 'static,
    ) -> Result<CasBulkWriteResponse> {
        let url = self.base.join("api/v1/cas/bulk/write")?;
        let (reader, writer) = piper::pipe(64 * 1024);
        let writer = tokio::task::spawn(async move {
            let mut tar = async_tar::Builder::new(writer);
            while let Some((key, content)) = entries.next().await {
                let mut header = async_tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, key.as_str(), content.as_slice())
                    .await
                    .with_context(|| format!("add entry: {key}"))?;
            }

            let mut writer = tar.into_inner().await.context("finalize tarball")?;
            writer.close().await.context("close writer")
        });

        let stream = ReaderStream::with_capacity(reader.compat(), 1024 * 1024);
        let body = reqwest::Body::wrap_stream(stream);
        let response = self
            .http
            .post(url)
            .header("Content-Type", "application/x-tar")
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
    #[instrument(name = "Courier::cas_read_bulk", skip(keys))]
    pub async fn cas_read_bulk(
        &self,
        keys: impl IntoIterator<Item = impl Into<Blake3>>,
    ) -> Result<impl Stream<Item = Result<(Blake3, Vec<u8>)>> + Unpin> {
        let url = self.base.join("api/v1/cas/bulk/read")?;
        let request = CasBulkReadRequest {
            keys: keys.into_iter().map(Into::into).collect(),
        };
        let response = self
            .http
            .post(url)
            .json(&request)
            .send()
            .await
            .context("send")?;

        let archive = response
            .bytes_stream()
            .map_err(std::io::Error::other)
            .pipe(StreamReader::new)
            .pipe(|r| Archive::new(r.compat()));

        let (tx, rx) = flume::bounded::<Result<(Blake3, Vec<u8>)>>(0);
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
                    let key = Blake3::from_hex_string(path.to_string_lossy())
                        .with_context(|| format!("parse entry name {path:?}"))?;

                    let mut content = Vec::new();
                    tokio::io::copy(&mut entry.compat(), &mut content)
                        .await
                        .context("read content")?;

                    tx.send_async(Ok((key, content)))
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
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CasBulkWriteResponse {
    pub written: HashSet<Blake3>,
    pub skipped: HashSet<Blake3>,
    pub errors: HashSet<CasBulkWriteKeyError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct CasBulkWriteKeyError {
    pub key: Blake3,
    pub error: String,
}

#[derive(Debug, Serialize)]
struct CasBulkReadRequest {
    keys: Vec<Blake3>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct ArtifactFile {
    pub object_key: String,
    pub mtime_nanos: u128,
    pub executable: bool,

    #[builder(into)]
    pub path: SerializeString<QualifiedPath>,
}

/// Serializes and deserializes the inner type to a JSON-encoded string.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, From, Deref)]
pub struct SerializeString<T>(T);

impl<T: Serialize> Serialize for SerializeString<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let inner = serde_json::to_string(&self.0).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&inner)
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for SerializeString<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let inner = String::deserialize(deserializer)?;
        serde_json::from_str(&inner)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct CargoSaveRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
    pub content_hash: String,
    pub artifacts: Vec<ArtifactFile>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct CargoRestoreRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct CargoRestoreResponse {
    pub artifacts: Vec<ArtifactFile>,
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
