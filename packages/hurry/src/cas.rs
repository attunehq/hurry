use std::{convert::identity, fmt::Debug};

use color_eyre::{Result, eyre::OptionExt};
use derive_more::Display;
use tracing::{debug, instrument};
use url::Url;

use crate::{client::Courier, hash::Blake3};

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

    /// Create a new instance with the provided base url.
    /// Instantiates a new [`Courier`] instance.
    pub fn new_client(base: Url) -> Self {
        Self {
            client: Courier::new(base),
        }
    }

    /// Store the entry in the CAS.
    /// Returns the key and whether the content was actually uploaded (true) or
    /// already existed (false).
    #[instrument(name = "CourierCas::store", skip(content))]
    pub async fn store(&self, content: &[u8]) -> Result<(Blake3, bool)> {
        let key = Blake3::from_buffer(content);
        if self.client.cas_exists(&key).await.is_ok_and(identity) {
            return Ok((key, false));
        }

        self.client.cas_write_bytes(&key, content.to_vec()).await?;
        debug!(?key, bytes = ?content.len(), "stored content");
        Ok((key, true))
    }

    /// Get the entry out of the CAS.
    #[instrument(name = "CourierCas::get")]
    pub async fn get(&self, key: &Blake3) -> Result<Option<Vec<u8>>> {
        self.client.cas_read_bytes(key).await
    }

    /// Get the entry out of the CAS.
    /// Errors if the entry is not available.
    #[instrument(name = "CourierCas::get")]
    pub async fn must_get(&self, key: &Blake3) -> Result<Vec<u8>> {
        self.client
            .cas_read_bytes(key)
            .await?
            .ok_or_eyre("key does not exist")
    }
}
