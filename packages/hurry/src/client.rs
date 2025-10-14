use bon::Builder;
use color_eyre::{Result, eyre::Context};
use futures::TryStreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio_util::io::{ReaderStream, StreamReader};
use tracing::instrument;
use url::Url;

use crate::{ext::join_all, hash::Blake3};

/// Client for the Courier API.
#[derive(Clone, Debug)]
pub struct Courier {
    base: Url,
    http: reqwest::Client,
}

impl Courier {
    /// Create a new client with the given base URL.
    pub fn new(base: impl Into<Url>) -> Self {
        Self {
            base: base.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Check if a CAS object exists.
    #[instrument(skip(self))]
    pub async fn cas_exists(&self, key: &Blake3) -> Result<bool> {
        let url = self.base.join_all(["api", "v1", "cas", key.as_str()])?;
        let response = self
            .http
            .head(url)
            .send()
            .await
            .context("send HEAD request")?;

        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => Err(color_eyre::eyre::eyre!(
                "unexpected status code from cas_exists: {status}"
            )),
        }
    }

    /// Read a CAS object.
    #[instrument(skip(self))]
    pub async fn cas_read(&self, key: &Blake3) -> Result<impl AsyncRead + Unpin> {
        let url = self.base.join_all(["api", "v1", "cas", key.as_str()])?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("send GET request")?;

        match response.status() {
            StatusCode::OK => {
                let stream = response.bytes_stream().map_err(std::io::Error::other);
                Ok(StreamReader::new(stream))
            }
            StatusCode::NOT_FOUND => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "CAS object not found",
            ))
            .context("cas_read"),
            status => Err(color_eyre::eyre::eyre!(
                "unexpected status code from cas_read: {status}"
            )),
        }
    }

    /// Write a CAS object.
    #[instrument(skip(self, content))]
    pub async fn cas_write(
        &self,
        key: &Blake3,
        content: impl AsyncRead + Unpin + Send + 'static,
    ) -> Result<()> {
        let url = self.base.join_all(["api", "v1", "cas", key.as_str()])?;
        let stream = ReaderStream::new(content);
        let body = reqwest::Body::wrap_stream(stream);

        let response = self
            .http
            .put(url)
            .body(body)
            .send()
            .await
            .context("send PUT request")?;

        match response.status() {
            StatusCode::CREATED => Ok(()),
            status => {
                let error_body = response.text().await.unwrap_or_default();
                Err(color_eyre::eyre::eyre!(
                    "unexpected status code from cas_write: {status}\n{error_body}"
                ))
            }
        }
    }

    /// Save cargo cache metadata.
    #[instrument(skip(self))]
    pub async fn cargo_cache_save(&self, request: CargoSaveRequest) -> Result<()> {
        let url = self
            .base
            .join_all(["api", "v1", "cache", "cargo", "save"])?;
        let response = self
            .http
            .post(url)
            .json(&request)
            .send()
            .await
            .context("send POST request")?;

        match response.status() {
            StatusCode::CREATED => Ok(()),
            status => {
                let error_body = response.text().await.unwrap_or_default();
                Err(color_eyre::eyre::eyre!(
                    "unexpected status code from cargo_cache_save: {status}\n{error_body}"
                ))
            }
        }
    }

    /// Restore cargo cache metadata.
    #[instrument(skip(self))]
    pub async fn cargo_cache_restore(
        &self,
        request: CargoRestoreRequest,
    ) -> Result<Option<CargoRestoreResponse>> {
        let url = self
            .base
            .join_all(["api", "v1", "cache", "cargo", "restore"])?;
        let response = self
            .http
            .post(url)
            .json(&request)
            .send()
            .await
            .context("send POST request")?;

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
                let error_body = response.text().await.unwrap_or_default();
                Err(color_eyre::eyre::eyre!(
                    "unexpected status code from cargo_cache_restore: {status}\n{error_body}"
                ))
            }
        }
    }

    /// Read a CAS object into a writer.
    pub async fn cas_read_into(
        &self,
        key: &Blake3,
        mut writer: impl AsyncWrite + Unpin,
    ) -> Result<()> {
        let mut reader = self.cas_read(key).await?;
        tokio::io::copy(&mut reader, &mut writer)
            .await
            .context("copy CAS content to writer")?;
        Ok(())
    }

    /// Write a CAS object from bytes.
    pub async fn cas_write_bytes(&self, key: &Blake3, content: Vec<u8>) -> Result<()> {
        let cursor = std::io::Cursor::new(content);
        self.cas_write(key, cursor).await
    }

    /// Read a CAS object into a byte vector.
    pub async fn cas_read_bytes(&self, key: &Blake3) -> Result<Vec<u8>> {
        let mut reader = self.cas_read(key).await?;
        let mut buffer = Vec::new();
        reader
            .read_to_end(&mut buffer)
            .await
            .context("read CAS content to bytes")?;
        Ok(buffer)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct ArtifactFile {
    pub object_key: String,
    pub path: String,
    pub mtime_nanos: u128,
    pub executable: bool,
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
