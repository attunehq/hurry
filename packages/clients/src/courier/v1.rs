//! Courier v1 API types and client.

use color_eyre::eyre::{Context, bail};
use derive_more::{Debug, Display};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing::{instrument, trace};

pub mod cache;
pub mod cas;

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
pub use client::Client;

/// The key to a content-addressed storage blob.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display)]
#[display("{}", self.to_hex())]
#[debug("{}", self.to_hex())]
pub struct Key(Vec<u8>);

impl Key {
    /// View the key as a hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Attempt to parse the key from a hex string.
    #[instrument(fields(hex = hex.as_ref()))]
    pub fn from_hex(hex: impl AsRef<str>) -> color_eyre::Result<Self> {
        let bytes = hex::decode(hex.as_ref()).context("decode hex")?;
        let len = bytes.len();
        trace!(?bytes, ?len, "decoded hex");
        if len != 32 {
            bail!("invalid hash length");
        }
        Ok(Self(bytes))
    }

    /// View the key as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Parse a key from raw bytes (the inverse of `as_bytes`).
    ///
    /// This is used when deserializing keys from the database or other binary
    /// formats. The bytes must be exactly 32 bytes (a blake3 hash).
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> color_eyre::Result<Self> {
        let bytes = bytes.as_ref();
        let len = bytes.len();
        if len != 32 {
            bail!("invalid hash length: expected 32 bytes, got {len}");
        }
        Ok(Self(bytes.to_vec()))
    }

    /// Create a key from a blake3 hash.
    pub fn from_blake3(hash: blake3::Hash) -> Self {
        Self(hash.as_bytes().to_vec())
    }

    /// Hash the contents of a buffer to create a key.
    ///
    /// This computes the blake3 hash of the provided buffer and returns the
    /// resulting key. Use this when you have file contents or other data
    /// that you want to content-address. This is NOT for parsing keys that
    /// are already in binary format: use `from_bytes` for that.
    pub fn from_buffer(buffer: impl AsRef<[u8]>) -> Self {
        let buffer = buffer.as_ref();
        let mut hasher = blake3::Hasher::new();
        hasher.update(buffer);
        let hash = hasher.finalize();
        Self::from_blake3(hash)
    }

    /// Hash the contents of the iterator in order.
    pub fn from_fields(fields: impl IntoIterator<Item = impl AsRef<[u8]>>) -> Self {
        let mut hasher = blake3::Hasher::new();
        for field in fields {
            hasher.update(field.as_ref());
        }
        let hash = hasher.finalize();
        Self::from_blake3(hash)
    }
}

impl From<&Key> for Key {
    fn from(key: &Key) -> Self {
        key.clone()
    }
}

impl PartialEq<blake3::Hash> for Key {
    fn eq(&self, other: &blake3::Hash) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<blake3::Hash> for &Key {
    fn eq(&self, other: &blake3::Hash) -> bool {
        self.0 == other.as_bytes()
    }
}

impl Serialize for Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex = String::deserialize(deserializer)?;
        Self::from_hex(&hex).map_err(serde::de::Error::custom)
    }
}
