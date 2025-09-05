//! Contains implementations of caches, and related infrastructure like CAS.

use bon::Builder;
use color_eyre::Result;
use enum_assoc::Assoc;
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, future::Future, path::Path};
use strum::Display;

use crate::{fs::Metadata, hash::Blake3};

mod fs;
pub use fs::*;

/// Conceptualizes file caching across providers and codebases.
pub trait Cache {
    /// Store the record in the cache.
    fn store(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + Debug + Send,
        artifacts: impl IntoIterator<Item = impl Into<Artifact>> + Debug + Send,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Get the record from the cache, if it exists.
    fn get(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + Debug + Send,
    ) -> impl Future<Output = Result<Option<Record>>> + Send;
}

impl<T: Cache + Sync> Cache for &T {
    async fn store(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + Debug + Send,
        artifacts: impl IntoIterator<Item = impl Into<Artifact>> + Debug + Send,
    ) -> Result<()> {
        Cache::store(*self, kind, key, artifacts).await
    }

    async fn get(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + Debug + Send,
    ) -> Result<Option<Record>> {
        Cache::get(*self, kind, key).await
    }
}

/// Conceptualizes "content addressed storage" across providers.
pub trait Cas {
    /// Store the content at the provided local file path in the CAS.
    /// Returns the key by which this content can be referred to in the future.
    fn store_file(
        &self,
        kind: Kind,
        file: impl AsRef<Path> + Debug + Send,
    ) -> impl Future<Output = Result<Blake3>> + Send;

    /// Get the content from the cache, if it exists,
    /// and write it to the output location.
    fn get_file(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + Debug + Send,
        destination: impl AsRef<Path> + Debug + Send,
    ) -> impl Future<Output = Result<()>> + Send;
}

impl<T: Cas + Sync> Cas for &T {
    async fn store_file(&self, kind: Kind, src: impl AsRef<Path> + Debug + Send) -> Result<Blake3> {
        Cas::store_file(*self, kind, src).await
    }

    async fn get_file(
        &self,
        kind: Kind,
        key: impl AsRef<Blake3> + Debug + Send,
        destination: impl AsRef<Path> + Debug + Send,
    ) -> Result<()> {
        Cas::get_file(*self, kind, key, destination).await
    }
}

/// The kind of project corresponding to the cache and CAS.
///
/// Generally, prefer naming these by build system rather than by language,
/// since most languages have more than one build system and the build systems
/// are really what matters for caching.
#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Deserialize, Serialize, Assoc,
)]
#[serde(rename_all = "snake_case")]
#[func(pub const fn as_str(&self) -> &str)]
pub enum Kind {
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
    pub kind: Kind,

    /// The cache key for this record.
    #[builder(into)]
    pub key: Blake3,

    /// The artifacts in this record.
    #[builder(default, into)]
    pub artifacts: Vec<Artifact>,
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

/// A recorded cache artifact.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize, Builder)]
pub struct Artifact {
    /// The target path for the artifact.
    ///
    /// This is expected to be relative to the "cache root" for the project;
    /// what specifically the "cache root" is depends on the project type
    /// but is by default the root of the project.
    #[builder(into)]
    pub target: RelativePathBuf,

    /// The hash of the content of the artifact.
    ///
    /// This is used to find the artifact data in the CAS.
    #[builder(into)]
    pub hash: Blake3,

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

impl From<&Artifact> for Artifact {
    fn from(value: &Artifact) -> Self {
        value.clone()
    }
}

impl AsRef<Artifact> for Artifact {
    fn as_ref(&self) -> &Artifact {
        self
    }
}
