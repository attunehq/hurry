//! Hashing operations and types.

use std::path::Path;

use color_eyre::Result;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use tracing::{instrument, trace};

use crate::fs;

/// A Blake3 hash.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Serialize, Deserialize)]
pub struct Blake3(String);

impl Blake3 {
    /// Hash the contents of the file at the specified path.
    #[instrument(name = "Blake3::from_file")]
    pub fn from_file(path: impl AsRef<Path> + std::fmt::Debug) -> Result<Self> {
        let path = path.as_ref();
        let file = fs::open_file(path)?;

        let mut reader = std::io::BufReader::new(file);
        let mut hasher = blake3::Hasher::new();
        let bytes = std::io::copy(&mut reader, &mut hasher)?;

        let hash = hasher.finalize().as_bytes().to_vec();
        let hash = hex::encode(hash);
        trace!(?path, ?hash, ?bytes, "hash file");
        Ok(Self(hash))
    }

    /// Hash the contents of a buffer.
    #[instrument(skip_all, name = "Blake3::from_buffer")]
    pub fn from_buffer(buffer: impl AsRef<[u8]> + std::fmt::Debug) -> Self {
        let buffer = buffer.as_ref();
        let mut hasher = blake3::Hasher::new();
        hasher.update(buffer);

        let hash = hasher.finalize().as_bytes().to_vec();
        let hash = hex::encode(hash);
        trace!(?hash, bytes = ?buffer.len(), "hash buffer");
        Self(hash)
    }

    /// Hash the contents of the iterator in order.
    #[instrument(skip_all, name = "Blake3::from_fields")]
    pub fn from_fields(fields: impl IntoIterator<Item = impl AsRef<[u8]>>) -> Self {
        let mut hasher = blake3::Hasher::new();
        let mut bytes = 0;

        for field in fields {
            let field = field.as_ref();
            bytes += field.len();
            hasher.update(field);
        }

        let hash = hasher.finalize().as_bytes().to_vec();
        let hash = hex::encode(hash);
        trace!(?hash, ?bytes, "hash fields");
        Self(hash)
    }

    /// View the hash as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&Blake3> for Blake3 {
    fn from(hash: &Blake3) -> Self {
        hash.clone()
    }
}

impl AsRef<str> for Blake3 {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<[u8]> for Blake3 {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl AsRef<Blake3> for Blake3 {
    fn as_ref(&self) -> &Blake3 {
        self
    }
}
