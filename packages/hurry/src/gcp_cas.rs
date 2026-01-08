//! GCP Cloud Storage-based content-addressed storage.
//!
//! This module provides a CAS implementation that stores blobs directly in a GCP
//! Cloud Storage bucket, bypassing the need for a Courier server. It also handles
//! unit metadata storage for a fully serverless cache solution.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use clients::courier::v1::{Key, SavedUnit, cache::CargoRestoreResponse};
use cloud_storage::Client;
use color_eyre::{Result, eyre::Context};
use derive_more::{Debug, Display};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, trace, warn};

/// Stored unit metadata in GCS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUnit {
    pub unit: SavedUnit,
    pub resolved_target: String,
    pub linux_glibc_version: Option<GlibcVersion>,
}

/// Simple glibc version for filtering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlibcVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl std::str::FromStr for GlibcVersion {
    type Err = color_eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('.');
        let major = parts
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("missing major version"))?
            .parse()?;
        let minor = parts
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("missing minor version"))?
            .parse()?;
        let patch = parts.next().map(|s| s.parse()).unwrap_or(Ok(0))?;
        Ok(Self { major, minor, patch })
    }
}

/// Check if an error is a 404 Not Found error.
fn is_not_found_error(e: &cloud_storage::Error) -> bool {
    match e {
        cloud_storage::Error::Google(google_err) => google_err.error.code == 404,
        _ => false,
    }
}

/// The remote content-addressed storage area backed by GCP Cloud Storage.
#[derive(Clone, Debug, Display)]
#[display("GcpCas(bucket={})", bucket)]
pub struct GcpCas {
    client: Arc<Client>,
    bucket: String,
}

impl GcpCas {
    /// Create a new instance with the given bucket name.
    ///
    /// Authentication is handled via the standard GCP authentication chain:
    /// - SERVICE_ACCOUNT environment variable (path to service account JSON)
    /// - gcloud CLI credentials
    /// - GCE metadata service (when running on GCP)
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            client: Arc::new(Client::default()),
            bucket: bucket.into(),
        }
    }

    /// Get the bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Get the object path for a given CAS key.
    /// Uses a two-level directory structure like the Courier server does:
    /// cas/first two hex chars/next two hex chars/full hex key
    fn cas_object_path(&self, key: &Key) -> String {
        let hex = key.to_hex();
        format!("cas/{}/{}/{}", &hex[0..2], &hex[2..4], hex)
    }

    /// Get the object path for unit metadata.
    /// units/{unit_hash}.json
    fn unit_object_path(&self, unit_hash: &str) -> String {
        format!("units/{}.json", unit_hash)
    }

    /// Check if the service is reachable by listing the bucket.
    #[instrument(skip(self))]
    pub async fn ping(&self) -> Result<()> {
        // Try to get bucket metadata to verify access
        self.client
            .bucket()
            .read(&self.bucket)
            .await
            .context("ping GCS bucket")?;
        info!(bucket = %self.bucket, "GCS bucket accessible");
        Ok(())
    }

    // ========== CAS Operations ==========

    /// Store the entry in the CAS.
    /// Returns the key and whether the content was actually uploaded (true) or
    /// already existed (false).
    #[instrument(name = "GcpCas::store", skip(content))]
    pub async fn store(&self, content: &[u8]) -> Result<(Key, bool)> {
        let key = Key::from_buffer(content);
        self.store_with_key(&key, content).await.map(|written| (key, written))
    }

    /// Store content with a pre-computed key.
    #[instrument(name = "GcpCas::store_with_key", skip(content))]
    pub async fn store_with_key(&self, key: &Key, content: &[u8]) -> Result<bool> {
        let path = self.cas_object_path(key);

        // Check if object already exists
        if self.exists(key).await? {
            return Ok(false);
        }

        // Compress with zstd before uploading
        let compressed = zstd::bulk::compress(content, 0).context("compress content")?;

        self.client
            .object()
            .create(&self.bucket, compressed, &path, "application/octet-stream+zstd")
            .await
            .context("upload to GCS")?;

        debug!(?key, bytes = ?content.len(), "stored content");
        Ok(true)
    }

    /// Check if an object exists in the bucket.
    #[instrument(name = "GcpCas::exists", skip(self))]
    pub async fn exists(&self, key: &Key) -> Result<bool> {
        let path = self.cas_object_path(key);
        match self.client.object().read(&self.bucket, &path).await {
            Ok(_) => Ok(true),
            Err(ref e) if is_not_found_error(e) => Ok(false),
            Err(e) => Err(e).context("check object existence"),
        }
    }

    /// Get the entry out of the CAS.
    #[instrument(name = "GcpCas::get")]
    pub async fn get(&self, key: &Key) -> Result<Option<Vec<u8>>> {
        let path = self.cas_object_path(key);

        match self.client.object().download(&self.bucket, &path).await {
            Ok(compressed) => {
                // Decompress the content (convert Box<[u8]> to Vec<u8>)
                let compressed: Vec<u8> = compressed.into();
                let decompressed = zstd::bulk::decompress(&compressed, 1024 * 1024 * 1024)
                    .context("decompress content")?;
                Ok(Some(decompressed))
            }
            Err(ref e) if is_not_found_error(e) => Ok(None),
            Err(e) => Err(e).context("download from GCS"),
        }
    }

    /// Get the entry out of the CAS.
    /// Errors if the entry is not available.
    #[instrument(name = "GcpCas::must_get")]
    pub async fn must_get(&self, key: &Key) -> Result<Vec<u8>> {
        self.get(key)
            .await?
            .ok_or_else(|| color_eyre::eyre::eyre!("key does not exist: {}", key))
    }

    /// Store multiple entries in the CAS via bulk write.
    #[instrument(name = "GcpCas::store_bulk", skip(entries))]
    pub async fn store_bulk(
        &self,
        mut entries: impl Stream<Item = (Key, Vec<u8>)> + Unpin + Send + 'static,
    ) -> Result<BulkStoreResult> {
        use futures::StreamExt;

        let mut written = BTreeSet::new();
        let mut skipped = BTreeSet::new();
        let mut errors = BTreeSet::new();

        while let Some((key, content)) = entries.next().await {
            match self.store_with_key(&key, &content).await {
                Ok(was_written) => {
                    if was_written {
                        written.insert(key);
                    } else {
                        skipped.insert(key);
                    }
                }
                Err(e) => {
                    errors.insert(BulkStoreError {
                        key,
                        error: e.to_string(),
                    });
                }
            }
        }

        Ok(BulkStoreResult {
            written,
            skipped,
            errors,
        })
    }

    /// Get multiple entries from the CAS via bulk read.
    #[instrument(name = "GcpCas::get_bulk", skip(keys))]
    pub async fn get_bulk(
        &self,
        keys: impl IntoIterator<Item = impl Into<Key>>,
    ) -> Result<impl Stream<Item = Result<(Key, Vec<u8>)>> + Unpin> {
        let keys: Vec<Key> = keys.into_iter().map(Into::into).collect();
        let client = self.client.clone();
        let bucket = self.bucket.clone();

        let (tx, rx) = flume::bounded::<Result<(Key, Vec<u8>)>>(0);

        tokio::task::spawn(async move {
            for key in keys {
                let path = {
                    let hex = key.to_hex();
                    format!("cas/{}/{}/{}", &hex[0..2], &hex[2..4], hex)
                };

                let result = match client.object().download(&bucket, &path).await {
                    Ok(compressed) => {
                        let compressed: Vec<u8> = compressed.into();
                        match zstd::bulk::decompress(&compressed, 1024 * 1024 * 1024) {
                            Ok(decompressed) => Ok((key, decompressed)),
                            Err(e) => Err(color_eyre::eyre::eyre!("decompress error: {}", e)),
                        }
                    }
                    Err(e) => Err(color_eyre::eyre::eyre!("download error: {}", e)),
                };

                if tx.send_async(result).await.is_err() {
                    break;
                }
            }
        });

        Ok(rx.into_stream())
    }

    // ========== Unit Metadata Operations ==========

    /// Save unit metadata to GCS.
    #[instrument(name = "GcpCas::save_unit", skip(self, unit))]
    pub async fn save_unit(
        &self,
        unit_hash: &str,
        unit: &SavedUnit,
        resolved_target: &str,
        linux_glibc_version: Option<&str>,
    ) -> Result<()> {
        let path = self.unit_object_path(unit_hash);

        let glibc_version = linux_glibc_version
            .map(|v| v.parse())
            .transpose()
            .context("parse glibc version")?;

        let stored = StoredUnit {
            unit: unit.clone(),
            resolved_target: resolved_target.to_string(),
            linux_glibc_version: glibc_version,
        };

        let json = serde_json::to_vec(&stored).context("serialize unit")?;

        self.client
            .object()
            .create(&self.bucket, json, &path, "application/json")
            .await
            .context("upload unit metadata to GCS")?;

        debug!(unit_hash, "saved unit metadata");
        Ok(())
    }

    /// Load unit metadata from GCS.
    #[instrument(name = "GcpCas::get_unit", skip(self))]
    pub async fn get_unit(&self, unit_hash: &str) -> Result<Option<StoredUnit>> {
        let path = self.unit_object_path(unit_hash);

        match self.client.object().download(&self.bucket, &path).await {
            Ok(data) => {
                let data: Vec<u8> = data.into();
                let stored: StoredUnit = serde_json::from_slice(&data)
                    .context("deserialize unit")?;
                Ok(Some(stored))
            }
            Err(ref e) if is_not_found_error(e) => Ok(None),
            Err(e) => Err(e).context("download unit metadata from GCS"),
        }
    }

    /// Restore multiple units, filtering by glibc version if specified.
    #[instrument(name = "GcpCas::restore_units", skip(self, unit_hashes))]
    pub async fn restore_units(
        &self,
        unit_hashes: impl IntoIterator<Item = impl AsRef<str>>,
        host_glibc_version: Option<&str>,
    ) -> Result<CargoRestoreResponse> {
        let host_glibc: Option<GlibcVersion> = host_glibc_version
            .map(|v| v.parse())
            .transpose()
            .context("parse host glibc version")?;

        let mut units = HashMap::new();
        let unit_hashes: Vec<_> = unit_hashes.into_iter().collect();
        let requested_count = unit_hashes.len();

        for unit_hash in unit_hashes {
            let unit_hash = unit_hash.as_ref();
            trace!(unit_hash, "fetching unit from GCS");

            match self.get_unit(unit_hash).await {
                Ok(Some(stored)) => {
                    // Filter by glibc version if both are specified
                    if let (Some(host), Some(unit_glibc)) = (&host_glibc, &stored.linux_glibc_version) {
                        if unit_glibc > host {
                            debug!(
                                unit_hash,
                                unit_glibc = %unit_glibc,
                                host_glibc = %host,
                                "skipping unit: requires newer glibc"
                            );
                            continue;
                        }
                    }

                    units.insert(stored.unit.unit_hash().clone(), stored.unit);
                }
                Ok(None) => {
                    trace!(unit_hash, "unit not found in cache");
                }
                Err(e) => {
                    warn!(unit_hash, error = %e, "failed to fetch unit");
                }
            }
        }

        info!(
            requested = requested_count,
            returned = units.len(),
            "GCS restore response"
        );

        Ok(CargoRestoreResponse::new(units))
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct BulkStoreResult {
    pub written: BTreeSet<Key>,
    pub skipped: BTreeSet<Key>,
    pub errors: BTreeSet<BulkStoreError>,
}

#[derive(Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Debug)]
pub struct BulkStoreError {
    pub key: Key,
    pub error: String,
}
