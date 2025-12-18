//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::From;
use serde::{Deserialize, Serialize};

use crate::courier::v1::{GlibcVersion, SavedUnit, SavedUnitHash};

/// A single `SavedUnit` and its associated cache key in a save request.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct CargoSaveUnitRequest {
    /// The `SavedUnit` to save.
    #[builder(into)]
    pub unit: SavedUnit,
    pub resolved_target: String,
    pub linux_glibc_version: Option<GlibcVersion>,
}

/// Request to save cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CargoSaveRequest {
    units: HashSet<CargoSaveUnitRequest>,
}

impl CargoSaveRequest {
    /// Create a new instance from the provided units.
    pub fn new(units: impl IntoIterator<Item = impl Into<CargoSaveUnitRequest>>) -> Self {
        let units = units.into_iter().map(Into::into).collect::<HashSet<_>>();
        Self { units }
    }

    /// Iterate over the units in the request.
    pub fn iter(&self) -> impl Iterator<Item = &CargoSaveUnitRequest> {
        self.units.iter()
    }
}

impl IntoIterator for CargoSaveRequest {
    type Item = CargoSaveUnitRequest;
    type IntoIter = std::collections::hash_set::IntoIter<CargoSaveUnitRequest>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.into_iter()
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
pub struct CargoRestoreRequest {
    pub units: HashSet<SavedUnitHash>,
    pub host_glibc_version: Option<GlibcVersion>,
}

impl CargoRestoreRequest {
    /// Create a new instance from the provided hashes.
    pub fn new(
        units: impl IntoIterator<Item = impl Into<SavedUnitHash>>,
        host_glibc_version: Option<GlibcVersion>,
    ) -> Self {
        let units = units.into_iter().map(Into::into).collect::<HashSet<_>>();
        Self {
            units,
            host_glibc_version,
        }
    }

    /// Iterate over the hashes in the request.
    pub fn iter(&self) -> impl Iterator<Item = &SavedUnitHash> {
        self.units.iter()
    }
}

impl IntoIterator for CargoRestoreRequest {
    type Item = SavedUnitHash;
    type IntoIter = std::collections::hash_set::IntoIter<SavedUnitHash>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.into_iter()
    }
}

impl From<&CargoRestoreRequest> for CargoRestoreRequest {
    fn from(req: &CargoRestoreRequest) -> Self {
        req.clone()
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From, Default)]
pub struct CargoRestoreResponse {
    units: HashMap<SavedUnitHash, SavedUnit>,
}

impl CargoRestoreResponse {
    /// Create a new instance from the provided hashes.
    pub fn new<I, H, U>(units: I) -> Self
    where
        I: IntoIterator<Item = (H, U)>,
        H: Into<SavedUnitHash>,
        U: Into<SavedUnit>,
    {
        let units = units
            .into_iter()
            .map(|(hash, unit)| (hash.into(), unit.into()))
            .collect::<HashMap<_, _>>();
        Self { units }
    }

    /// Iterate over the units in the response.
    pub fn iter(&self) -> impl Iterator<Item = (&SavedUnitHash, &SavedUnit)> {
        self.units.iter()
    }

    /// Check if the response is empty.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Get the number of units in the response.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Get a unit by its cache key.
    pub fn get(&self, key: &SavedUnitHash) -> Option<&SavedUnit> {
        self.units.get(key)
    }

    /// Consume a unit by its cache key, removing it from the response.
    pub fn take(&mut self, key: &SavedUnitHash) -> Option<SavedUnit> {
        self.units.remove(key)
    }
}

impl IntoIterator for CargoRestoreResponse {
    type Item = (SavedUnitHash, SavedUnit);
    type IntoIter = std::collections::hash_map::IntoIter<SavedUnitHash, SavedUnit>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.into_iter()
    }
}

impl FromIterator<(SavedUnitHash, SavedUnit)> for CargoRestoreResponse {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (SavedUnitHash, SavedUnit)>,
    {
        Self {
            units: iter.into_iter().collect(),
        }
    }
}

impl From<&CargoRestoreResponse> for CargoRestoreResponse {
    fn from(resp: &CargoRestoreResponse) -> Self {
        resp.clone()
    }
}
