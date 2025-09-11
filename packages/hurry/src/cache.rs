//! Contains implementations of caches, and related infrastructure like CAS.

use bon::Builder;
use enum_assoc::Assoc;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use strum::Display;

use crate::{fs::Metadata, hash::Blake3, path::RelFilePath};

mod fs;
pub use fs::*;

/// The kind of project represented by a cache [`Record`].
///
/// Generally, prefer naming these by build system rather than by language,
/// since most languages have more than one build system and the build systems
/// are really what matters for caching.
#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Deserialize, Serialize, Assoc,
)]
#[serde(rename_all = "snake_case")]
#[func(pub const fn as_str(&self) -> &str)]
pub enum RecordKind {
    /// A Rust project managed by Cargo.
    #[assoc(as_str = "cargo")]
    Cargo,
}

/// A record of artifacts in the cache for a given key.
///
/// The idea here is that a given key can have one or more attached
/// artifacts; looking up a key returns the list of all artifacts
/// in that key (which can be further pared down if desired).
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize, Builder)]
pub struct Record {
    /// The kind of project being cached.
    #[builder(into)]
    pub kind: RecordKind,

    /// The cache key for this record.
    #[builder(into)]
    pub key: Blake3,

    /// The artifacts in this record.
    #[builder(default, into)]
    pub artifacts: Vec<RecordArtifact>,
}

impl From<&Record> for Record {
    fn from(value: &Record) -> Self {
        value.clone()
    }
}

impl AsRef<Record> for Record {
    fn as_ref(&self) -> &Record {
        self
    }
}

/// A recorded artifact in a cache [`Record`].
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize, Builder)]
pub struct RecordArtifact {
    /// The target path from which the artifact was backed up (and therefore,
    /// to which the artifact should be restored).
    ///
    /// This is expected to be relative to the "cache root" for the project;
    /// what specifically the "cache root" is depends on the project type
    /// but is by default the root of the project.
    #[builder(into)]
    pub target: RelFilePath,

    /// The CAS key for the content of the artifact.
    /// This is the [`Blake3`] of the content.
    #[builder(into)]
    pub cas_key: Blake3,

    /// The file metadata of the artifact.
    ///
    /// When the artifact is restored from the CAS object in cache, this is used
    /// to restore metadata like the mtime and permissions. Note that we cannot
    /// simply leave the metadata on the CAS object because multiple artifacts
    /// may map to the same CAS object (e.g. all files of size zero are the same
    /// object).
    #[builder(into)]
    pub metadata: Metadata,
}

impl From<&RecordArtifact> for RecordArtifact {
    fn from(value: &RecordArtifact) -> Self {
        value.clone()
    }
}

impl AsRef<RecordArtifact> for RecordArtifact {
    fn as_ref(&self) -> &RecordArtifact {
        self
    }
}
