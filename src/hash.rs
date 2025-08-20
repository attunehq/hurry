//! Hashing operations and types.

use std::{io::Read, path::Path};

use color_eyre::Result;
use color_eyre::eyre::Context;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// A Blake3 hash.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Serialize, Deserialize)]
pub struct Blake3(String);

impl Blake3 {
    /// Hash the contents of the file at the specified path.
    #[instrument]
    pub fn from_file(path: impl AsRef<Path> + std::fmt::Debug) -> Result<Self> {
        let path = path.as_ref();
        let file = std::fs::File::open(path).with_context(|| format!("open {path:?}"))?;
        let reader = std::io::BufReader::new(file);
        Self::from_reader(reader)
    }

    /// Hash the contents of a buffer.
    #[instrument]
    pub fn from_buffer(buffer: impl AsRef<[u8]> + std::fmt::Debug) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(buffer.as_ref());
        let hash = hasher.finalize().as_bytes().to_vec();
        Self(hex::encode(hash))
    }

    /// Hash the contents of the reader.
    #[instrument(skip_all)]
    pub fn from_reader(mut reader: impl Read) -> Result<Self> {
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut reader, &mut hasher)?;
        let hash = hasher.finalize().as_bytes().to_vec();
        Ok(Self(hex::encode(hash)))
    }

    /// Hash the contents of the iterator in order.
    #[instrument(skip_all)]
    pub fn from_fields(fields: impl IntoIterator<Item = impl AsRef<[u8]>>) -> Self {
        let mut hasher = blake3::Hasher::new();
        for field in fields {
            hasher.update(field.as_ref());
        }
        let hash = hasher.finalize().as_bytes().to_vec();
        Self(hex::encode(hash))
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
