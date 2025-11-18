//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::From;
use serde::{Deserialize, Serialize};
use tap::Pipe;

use crate::courier::v1::{Key, SavedUnit, SavedUnitHash};

/// Compound cache key for `SavedUnit`.
///
/// Today, we only cache by `SavedUnitHash`, but soon we will add other fields
/// to the cache key such as libc version and possibly more.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct SavedUnitCacheKey {
    /// `SavedUnit` instances are primarily keyed by their hash.
    #[builder(into)]
    pub unit: SavedUnitHash,
}

impl SavedUnitCacheKey {
    /// Construct a single opaque string representing the cache key.
    ///
    /// The contents of this string should be treated as opaque: its format may
    /// change at any time. The only guaranteed quality of the returned value is
    /// that it will always be the same if the contents of the
    /// `SavedUnitCacheKey` instance are the same, and always different if the
    /// contents are different.
    ///
    /// Note: this is meant to be similar to a derived `Hash` implementation,
    /// but stable across compiler versions and platforms.
    pub fn stable_hash(&self) -> String {
        // When we add new fields, this will show a compile time error; if you got here
        // due to a compilation error please handle the new field(s) appropriately.
        let Self { unit } = self;
        let mut hasher = blake3::Hasher::new();
        hasher.update(unit.as_str().as_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

impl AsRef<SavedUnitHash> for SavedUnitCacheKey {
    fn as_ref(&self) -> &SavedUnitHash {
        &self.unit
    }
}

impl From<&SavedUnitCacheKey> for SavedUnitCacheKey {
    fn from(key: &SavedUnitCacheKey) -> Self {
        key.clone()
    }
}

/// A single `SavedUnit` and its associated cache key in a save request.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct CargoSaveUnitRequest {
    /// The cache key for the `SavedUnit` instance.
    #[builder(into)]
    pub key: SavedUnitCacheKey,

    /// The `SavedUnit` to save.
    #[builder(into)]
    pub unit: SavedUnit,
}

/// Request to save cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From)]
#[non_exhaustive]
pub struct CargoSaveRequest2(HashSet<CargoSaveUnitRequest>);

impl CargoSaveRequest2 {
    /// Create a new instance from the provided units.
    pub fn new(units: impl IntoIterator<Item = impl Into<CargoSaveUnitRequest>>) -> Self {
        units
            .into_iter()
            .map(Into::into)
            .collect::<HashSet<_>>()
            .pipe(Self)
    }

    /// Iterate over the units in the request.
    pub fn iter(&self) -> impl Iterator<Item = &CargoSaveUnitRequest> {
        self.0.iter()
    }
}

impl IntoIterator for CargoSaveRequest2 {
    type Item = CargoSaveUnitRequest;
    type IntoIter = std::collections::hash_set::IntoIter<CargoSaveUnitRequest>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<CargoSaveUnitRequest> for CargoSaveRequest2 {
    fn from_iter<T: IntoIterator<Item = CargoSaveUnitRequest>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl From<&CargoSaveRequest2> for CargoSaveRequest2 {
    fn from(req: &CargoSaveRequest2) -> Self {
        req.clone()
    }
}

/// Request to restore cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From)]
#[non_exhaustive]
pub struct CargoRestoreRequest2(HashSet<SavedUnitCacheKey>);

impl CargoRestoreRequest2 {
    /// Create a new instance from the provided hashes.
    pub fn new(units: impl IntoIterator<Item = impl Into<SavedUnitCacheKey>>) -> Self {
        units
            .into_iter()
            .map(Into::into)
            .collect::<HashSet<_>>()
            .pipe(Self)
    }

    /// Iterate over the hashes in the request.
    pub fn iter(&self) -> impl Iterator<Item = &SavedUnitCacheKey> {
        self.0.iter()
    }
}

impl IntoIterator for CargoRestoreRequest2 {
    type Item = SavedUnitCacheKey;
    type IntoIter = std::collections::hash_set::IntoIter<SavedUnitCacheKey>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<SavedUnitCacheKey> for CargoRestoreRequest2 {
    fn from_iter<T: IntoIterator<Item = SavedUnitCacheKey>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl From<&CargoRestoreRequest2> for CargoRestoreRequest2 {
    fn from(req: &CargoRestoreRequest2) -> Self {
        req.clone()
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From)]
#[non_exhaustive]
pub struct CargoRestoreResponse2(HashMap<SavedUnitCacheKey, SavedUnit>);

impl CargoRestoreResponse2 {
    /// Create a new instance from the provided hashes.
    pub fn new<I, H, U>(units: I) -> Self
    where
        I: IntoIterator<Item = (H, U)>,
        H: Into<SavedUnitCacheKey>,
        U: Into<SavedUnit>,
    {
        units
            .into_iter()
            .map(|(hash, unit)| (hash.into(), unit.into()))
            .collect::<HashMap<_, _>>()
            .pipe(Self)
    }

    /// Iterate over the units in the response.
    pub fn iter(&self) -> impl Iterator<Item = (&SavedUnitCacheKey, &SavedUnit)> {
        self.0.iter()
    }
}

impl IntoIterator for CargoRestoreResponse2 {
    type Item = (SavedUnitCacheKey, SavedUnit);
    type IntoIter = std::collections::hash_map::IntoIter<SavedUnitCacheKey, SavedUnit>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(SavedUnitCacheKey, SavedUnit)> for CargoRestoreResponse2 {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (SavedUnitCacheKey, SavedUnit)>,
    {
        iter.into_iter().collect()
    }
}

impl From<&CargoRestoreResponse2> for CargoRestoreResponse2 {
    fn from(resp: &CargoRestoreResponse2) -> Self {
        resp.clone()
    }
}

/// An artifact file in the cargo cache.
/// The path is stored as a JSON-encoded string.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
#[deprecated = "Replaced by `SavedFile`"]
pub struct ArtifactFile {
    pub mtime_nanos: u128,
    pub executable: bool,

    #[builder(into)]
    pub object_key: Key,

    #[builder(into)]
    pub path: String,
}

impl From<&ArtifactFile> for ArtifactFile {
    fn from(file: &ArtifactFile) -> Self {
        file.clone()
    }
}

/// Request to save cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
#[deprecated = "Replaced by `CargoSaveRequest2`"]
pub struct CargoSaveRequest {
    #[builder(into)]
    pub package_name: String,

    #[builder(into)]
    pub package_version: String,

    #[builder(into)]
    pub target: String,

    #[builder(into)]
    pub library_crate_compilation_unit_hash: String,

    #[builder(into)]
    pub build_script_compilation_unit_hash: Option<String>,

    #[builder(into)]
    pub build_script_execution_unit_hash: Option<String>,

    #[builder(into)]
    pub content_hash: String,

    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<ArtifactFile>>| i.into_iter().map(Into::into).collect())]
    pub artifacts: Vec<ArtifactFile>,
}

impl From<&CargoSaveRequest> for CargoSaveRequest {
    fn from(req: &CargoSaveRequest) -> Self {
        req.clone()
    }
}

/// Request to restore cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
#[deprecated = "Replaced by `CargoRestoreRequest2`"]
pub struct CargoRestoreRequest {
    #[builder(into)]
    pub package_name: String,

    #[builder(into)]
    pub package_version: String,

    #[builder(into)]
    pub target: String,

    #[builder(into)]
    pub library_crate_compilation_unit_hash: String,

    #[builder(into)]
    pub build_script_compilation_unit_hash: Option<String>,

    #[builder(into)]
    pub build_script_execution_unit_hash: Option<String>,
}

impl CargoRestoreRequest {
    pub fn hash(&self) -> Vec<u8> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.package_name.as_bytes());
        hasher.update(self.package_version.as_bytes());
        hasher.update(self.target.as_bytes());
        hasher.update(self.library_crate_compilation_unit_hash.as_bytes());
        if let Some(hash) = &self.build_script_compilation_unit_hash {
            hasher.update(hash.as_bytes());
        }
        if let Some(hash) = &self.build_script_execution_unit_hash {
            hasher.update(hash.as_bytes());
        }
        hasher.finalize().as_bytes().to_vec()
    }
}

impl From<&CargoRestoreRequest> for CargoRestoreRequest {
    fn from(req: &CargoRestoreRequest) -> Self {
        req.clone()
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
#[deprecated = "Replaced by `CargoRestoreResponse2`"]
pub struct CargoRestoreResponse {
    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<ArtifactFile>>| i.into_iter().map(Into::into).collect())]
    pub artifacts: Vec<ArtifactFile>,
}

impl From<&CargoRestoreResponse> for CargoRestoreResponse {
    fn from(resp: &CargoRestoreResponse) -> Self {
        resp.clone()
    }
}

/// Request to restore multiple cargo cache entries in bulk.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
#[deprecated = "Replaced by `CargoRestoreRequest2`"]
pub struct CargoBulkRestoreRequest {
    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<CargoRestoreRequest>>| i.into_iter().map(Into::into).collect())]
    pub requests: Vec<CargoRestoreRequest>,
}

impl From<&CargoBulkRestoreRequest> for CargoBulkRestoreRequest {
    fn from(req: &CargoBulkRestoreRequest) -> Self {
        req.clone()
    }
}

/// A single cache hit in a bulk restore operation.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
#[deprecated = "No longer used when we swap to `CargoRestoreRequest2`"]
pub struct CargoBulkRestoreHit {
    /// The original request that produced this hit
    pub request: CargoRestoreRequest,

    /// The artifacts for this cache entry
    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<ArtifactFile>>| i.into_iter().map(Into::into).collect())]
    pub artifacts: Vec<ArtifactFile>,
}

impl From<&CargoBulkRestoreHit> for CargoBulkRestoreHit {
    fn from(hit: &CargoBulkRestoreHit) -> Self {
        hit.clone()
    }
}

/// Response from bulk restore operation.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder, Default)]
#[non_exhaustive]
#[deprecated = "No longer used when we swap to `CargoRestoreRequest2`"]
pub struct CargoBulkRestoreResponse {
    /// Requests that had matching cache entries
    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<CargoBulkRestoreHit>>| i.into_iter().map(Into::into).collect())]
    pub hits: Vec<CargoBulkRestoreHit>,

    /// Requests that had no matching cache entry
    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<CargoRestoreRequest>>| i.into_iter().map(Into::into).collect())]
    pub misses: Vec<CargoRestoreRequest>,
}

impl From<&CargoBulkRestoreResponse> for CargoBulkRestoreResponse {
    fn from(resp: &CargoBulkRestoreResponse) -> Self {
        resp.clone()
    }
}
