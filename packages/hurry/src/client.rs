use std::sync::Arc;

use bon::Builder;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, eyre},
};
use derive_more::{Debug, Deref, Display, From};
use futures::TryStreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use tap::Pipe;
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};
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
    pub fn new(base: Url) -> Self {
        Self {
            base: Arc::new(base),
            http: reqwest::Client::new(),
        }
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
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
        let stream = ReaderStream::with_capacity(content, 64 * 1024);
        let body = reqwest::Body::wrap_stream(stream);

        let response = self.http.put(url).body(body).send().await.context("send")?;
        match response.status() {
            StatusCode::CREATED => Ok(()),
            status => {
                let url = response.url().to_string();
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
                let body = response.text().await.unwrap_or_default();
                return Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"));
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
                let body = response.text().await.unwrap_or_default();
                Err(eyre!("unexpected status code: {status}"))
                    .with_section(|| url.header("Url:"))
                    .with_section(|| body.header("Body:"))
            }
        }
    }
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
