//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::{AsRef, From};
use serde::{Deserialize, Serialize};

use crate::courier::v1::{ArtifactFile, SavedUnitHash, UnitSavePlan};

/// Request to save cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From, AsRef)]
#[non_exhaustive]
pub struct CargoSaveRequest(UnitSavePlan);

impl From<&CargoSaveRequest> for CargoSaveRequest {
    fn from(req: &CargoSaveRequest) -> Self {
        req.clone()
    }
}

/// Request to restore cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CargoRestoreRequest(HashSet<SavedUnitHash>);

impl From<&CargoRestoreRequest> for CargoRestoreRequest {
    fn from(req: &CargoRestoreRequest) -> Self {
        req.clone()
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From, AsRef)]
#[non_exhaustive]
pub struct CargoRestoreResponse(HashMap<SavedUnitHash, UnitSavePlan>);

impl From<&CargoRestoreResponse> for CargoRestoreResponse {
    fn from(resp: &CargoRestoreResponse) -> Self {
        resp.clone()
    }
}

/// A single cache hit in a bulk restore operation.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
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
