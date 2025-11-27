use std::{collections::BTreeSet, convert::identity, fmt::Debug};

use clients::{Courier, Token, courier::v1::Key};
use color_eyre::{Result, eyre::OptionExt};
use derive_more::Display;
use futures::Stream;
use tracing::{debug, instrument};
use url::Url;

/// The remote content-addressed storage area backed by Courier.
#[derive(Clone, Debug, Display)]
#[display("{client}")]
pub struct CourierCas {
    client: Courier,
}

impl CourierCas {
    /// Create a new instance with the given client.
    pub fn new(client: Courier) -> Self {
        Self { client }
    }

    /// Create a new instance with the provided base url and token.
    /// Instantiates a new [`Courier`] instance.
    pub fn new_client(base: Url, token: Token) -> Result<Self> {
        let client = Courier::new(base, token)?;
        Ok(Self { client })
    }

    /// Store the entry in the CAS.
    /// Returns the key and whether the content was actually uploaded (true) or
    /// already existed (false).
    #[instrument(name = "CourierCas::store", skip(content))]
    pub async fn store(&self, content: &[u8]) -> Result<(Key, bool)> {
        let key = Key::from_buffer(content);
        if self.client.cas_exists(&key).await.is_ok_and(identity) {
            return Ok((key, false));
        }

        self.client.cas_write_bytes(&key, content.to_vec()).await?;
        debug!(?key, bytes = ?content.len(), "stored content");
        Ok((key, true))
    }

    /// Get the entry out of the CAS.
    #[instrument(name = "CourierCas::get")]
    pub async fn get(&self, key: &Key) -> Result<Option<Vec<u8>>> {
        self.client.cas_read_bytes(key).await
    }

    /// Get the entry out of the CAS.
    /// Errors if the entry is not available.
    #[instrument(name = "CourierCas::get")]
    pub async fn must_get(&self, key: &Key) -> Result<Vec<u8>> {
        self.client
            .cas_read_bytes(key)
            .await?
            .ok_or_eyre("key does not exist")
    }

    /// Store multiple entries in the CAS via bulk write.
    #[instrument(name = "CourierCas::store_bulk", skip(entries))]
    pub async fn store_bulk(
        &self,
        entries: impl Stream<Item = (Key, Vec<u8>)> + Unpin + Send + 'static,
    ) -> Result<BulkStoreResult> {
        self.client
            .cas_write_bulk(entries)
            .await
            .map(|response| BulkStoreResult {
                written: response.written,
                skipped: response.skipped,
                errors: response
                    .errors
                    .into_iter()
                    .map(|item| BulkStoreError {
                        key: item.key,
                        error: item.error,
                    })
                    .collect(),
            })
    }

    /// Get multiple entries from the CAS via bulk read.
    #[instrument(name = "CourierCas::get_bulk", skip(keys))]
    pub async fn get_bulk(
        &self,
        keys: impl IntoIterator<Item = impl Into<Key>>,
    ) -> Result<impl Stream<Item = Result<(Key, Vec<u8>)>> + Unpin> {
        self.client.cas_read_bulk(keys).await
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
