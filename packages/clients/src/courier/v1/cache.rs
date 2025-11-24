//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::From;
use serde::{Deserialize, Serialize};
use tap::Pipe;

use crate::courier::v1::{SavedUnit, SavedUnitHash};

/// Compound cache key for `SavedUnit`.
///
/// Today, we only cache by `SavedUnitHash`, but soon we will add other fields
/// to the cache key such as libc version and possibly more.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct SavedUnitCacheKey {
    /// The key generation, used to invalidate keys that no longer point to the
    /// same content.
    #[builder(skip = SavedUnitCacheKey::GENERATION)]
    generation: u8,

    /// `SavedUnit` instances are primarily keyed by their hash.
    #[builder(into)]
    pub unit_hash: SavedUnitHash,
}

impl SavedUnitCacheKey {
    /// The current generation for the cache key.
    ///
    /// This exists so that if we _semantically_ change how the cache works
    /// without actually changing how the cache key is generated (so e.g. the
    /// same key means something different than it used to mean, or holds
    /// different content) we can increment the generation to force a change
    /// to the key.
    const GENERATION: u8 = 1;

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
        let Self {
            unit_hash,
            generation,
        } = self;
        let mut hasher = blake3::Hasher::new();
        hasher.update(format!("{generation}").as_bytes());
        hasher.update(unit_hash.as_str().as_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

impl AsRef<SavedUnitHash> for SavedUnitCacheKey {
    fn as_ref(&self) -> &SavedUnitHash {
        &self.unit_hash
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
pub struct CargoSaveRequest(HashSet<CargoSaveUnitRequest>);

impl CargoSaveRequest {
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

impl IntoIterator for CargoSaveRequest {
    type Item = CargoSaveUnitRequest;
    type IntoIter = std::collections::hash_set::IntoIter<CargoSaveUnitRequest>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<CargoSaveUnitRequest> for CargoSaveRequest {
    fn from_iter<T: IntoIterator<Item = CargoSaveUnitRequest>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl From<&CargoSaveRequest> for CargoSaveRequest {
    fn from(req: &CargoSaveRequest) -> Self {
        req.clone()
    }
}

/// Request to restore cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From)]
#[non_exhaustive]
pub struct CargoRestoreRequest(HashSet<SavedUnitCacheKey>);

impl CargoRestoreRequest {
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

impl IntoIterator for CargoRestoreRequest {
    type Item = SavedUnitCacheKey;
    type IntoIter = std::collections::hash_set::IntoIter<SavedUnitCacheKey>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<SavedUnitCacheKey> for CargoRestoreRequest {
    fn from_iter<T: IntoIterator<Item = SavedUnitCacheKey>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl From<&CargoRestoreRequest> for CargoRestoreRequest {
    fn from(req: &CargoRestoreRequest) -> Self {
        req.clone()
    }
}

/// Intermediate transport type used when requesting a restore.
///
/// JSON does not permit non-string keys in objects, and we would like to use
/// the struct `SavedUnitCacheKey` as a key in our response map. We work around
/// this by instead sending a list of (key, value) object pairs using this type
/// instead of CargoRestoreResponse, and parsing the list of keys and values
/// back into a map when received.
#[derive(Debug, Clone, Serialize, Deserialize, From)]
pub struct CargoRestoreResponseTransport(HashSet<(SavedUnitCacheKey, SavedUnit)>);

impl CargoRestoreResponseTransport {
    /// Iterate over the units in the response.
    pub fn iter(&self) -> impl Iterator<Item = (&SavedUnitCacheKey, &SavedUnit)> {
        // This looks odd, but it's sugar going from `&(A, B)` to `(&A, &B)`.
        self.0.iter().map(|(a, b)| (a, b))
    }
}

impl From<CargoRestoreResponseTransport> for CargoRestoreResponse {
    fn from(resp: CargoRestoreResponseTransport) -> Self {
        resp.into_iter().collect()
    }
}

impl IntoIterator for CargoRestoreResponseTransport {
    type Item = (SavedUnitCacheKey, SavedUnit);
    type IntoIter = std::collections::hash_set::IntoIter<(SavedUnitCacheKey, SavedUnit)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(SavedUnitCacheKey, SavedUnit)> for CargoRestoreResponseTransport {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (SavedUnitCacheKey, SavedUnit)>,
    {
        Self(iter.into_iter().collect())
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From)]
pub struct CargoRestoreResponse(HashMap<SavedUnitCacheKey, SavedUnit>);

impl CargoRestoreResponse {
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

    /// Check if the response is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get a unit by its cache key.
    pub fn get(&self, key: &SavedUnitCacheKey) -> Option<&SavedUnit> {
        self.0.get(key)
    }

    /// Consume a unit by its cache key, removing it from the response.
    pub fn take(&mut self, key: &SavedUnitCacheKey) -> Option<SavedUnit> {
        self.0.remove(key)
    }
}

impl IntoIterator for CargoRestoreResponse {
    type Item = (SavedUnitCacheKey, SavedUnit);
    type IntoIter = std::collections::hash_map::IntoIter<SavedUnitCacheKey, SavedUnit>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(SavedUnitCacheKey, SavedUnit)> for CargoRestoreResponse {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (SavedUnitCacheKey, SavedUnit)>,
    {
        Self(iter.into_iter().collect())
    }
}

impl From<&CargoRestoreResponse> for CargoRestoreResponse {
    fn from(resp: &CargoRestoreResponse) -> Self {
        resp.clone()
    }
}
