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
            .head(&url)
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
        let url = format!("{}/api/v1/cas/{}", self.base, key.as_string());
        let response = self
            .http
            .get(&url)
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
        let url = format!("{}/api/v1/cas/{}", self.base, key.as_string());
        let stream = ReaderStream::new(content);
        let body = reqwest::Body::wrap_stream(stream);

        let response = self
            .http
            .put(&url)
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
        let url = format!("{}/api/v1/cache/cargo/save", self.base);
        let response = self
            .http
            .post(&url)
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
        let url = format!("{}/api/v1/cache/cargo/restore", self.base);
        let response = self
            .http
            .post(&url)
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

#[cfg(test)]
mod tests {
    use super::*;
    use color_eyre::Result;
    use pretty_assertions::assert_eq as pretty_assert_eq;

    #[test_log::test(tokio::test)]
    #[ignore]
    async fn cas_write_read_roundtrip() -> Result<()> {
        let client = Courier::new("http://localhost:8080");
        let content = b"hello world from hurry client";
        let hash = Blake3::from_buffer(content);

        client.cas_write_bytes(&hash, content.to_vec()).await?;

        let exists = client.cas_exists(&hash).await?;
        pretty_assert_eq!(exists, true);

        let read_content = client.cas_read_bytes(&hash).await?;
        pretty_assert_eq!(read_content.as_slice(), content);

        Ok(())
    }

    #[test_log::test(tokio::test)]
    #[ignore]
    async fn cas_nonexistent() -> Result<()> {
        let client = Courier::new("http://localhost:8080");
        let hash = Blake3::from_buffer(b"nonexistent content");

        let exists = client.cas_exists(&hash).await?;
        pretty_assert_eq!(exists, false);

        let result = client.cas_read_bytes(&hash).await;
        assert!(result.is_err());

        Ok(())
    }

    #[test_log::test(tokio::test)]
    #[ignore]
    async fn cargo_cache_save_restore() -> Result<()> {
        let client = Courier::new("http://localhost:8080");

        let save_request = CargoSaveRequest::builder()
            .package_name("test-package")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("test_hash_123")
            .content_hash("content_hash_123")
            .artifacts(vec![
                ArtifactFile::builder()
                    .object_key("blake3_test_key")
                    .path("libtest.rlib")
                    .mtime_nanos(1234567890123456789)
                    .executable(false)
                    .build(),
            ])
            .build();

        client.cargo_cache_save(save_request).await?;

        let restore_request = CargoRestoreRequest::builder()
            .package_name("test-package")
            .package_version("1.0.0")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("test_hash_123")
            .build();

        let response = client.cargo_cache_restore(restore_request).await?;
        assert!(response.is_some());

        let response = response.unwrap();
        pretty_assert_eq!(response.artifacts.len(), 1);
        pretty_assert_eq!(response.artifacts[0].object_key, "blake3_test_key");
        pretty_assert_eq!(response.artifacts[0].path, "libtest.rlib");
        pretty_assert_eq!(response.artifacts[0].mtime_nanos, 1234567890123456789);
        pretty_assert_eq!(response.artifacts[0].executable, false);

        Ok(())
    }

    #[test_log::test(tokio::test)]
    #[ignore]
    async fn cargo_cache_restore_miss() -> Result<()> {
        let client = Courier::new("http://localhost:8080");

        let restore_request = CargoRestoreRequest::builder()
            .package_name("nonexistent-package")
            .package_version("99.99.99")
            .target("x86_64-unknown-linux-gnu")
            .library_crate_compilation_unit_hash("nonexistent_hash")
            .build();

        let response = client.cargo_cache_restore(restore_request).await?;
        pretty_assert_eq!(response, None);

        Ok(())
    }

    #[test_log::test(tokio::test)]
    #[ignore]
    async fn cas_streaming() -> Result<()> {
        let client = Courier::new("http://localhost:8080");
        const CONTENT: &[u8] = &[0xAB; 1024 * 1024]; // 1MB
        let hash = Blake3::from_buffer(CONTENT);

        let cursor = std::io::Cursor::new(CONTENT);
        client.cas_write(&hash, cursor).await?;

        let mut output = Vec::new();
        client.cas_read_into(&hash, &mut output).await?;

        pretty_assert_eq!(hex::encode(output), hex::encode(CONTENT));
        Ok(())
    }
}
