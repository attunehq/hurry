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
        Self::from_reader(file)
    }

    /// Hash the contents of a buffer.
    #[instrument]
    pub fn from_buffer(buffer: impl AsRef<[u8]> + std::fmt::Debug) -> Result<Self> {
        Self::from_reader(std::io::Cursor::new(buffer))
    }

    /// Hash the contents of the reader.
    #[instrument(skip_all)]
    pub fn from_reader(reader: impl Read) -> Result<Self> {
        let mut reader = std::io::BufReader::new(reader);
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut reader, &mut hasher)?;
        let hash = hasher.finalize().as_bytes().to_vec();
        Ok(Self(hex::encode(hash)))
    }

    /// View the hash as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
