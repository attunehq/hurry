//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::{AsRef, From};
use serde::{Deserialize, Serialize};

use crate::courier::v1::{Key, SavedUnitHash, UnitSavePlan};

/// Request to save cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From, AsRef)]
#[non_exhaustive]
pub struct CargoSaveRequest2(UnitSavePlan);

impl From<&CargoSaveRequest2> for CargoSaveRequest2 {
    fn from(req: &CargoSaveRequest2) -> Self {
        req.clone()
    }
}

/// Request to restore cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CargoRestoreRequest2(HashSet<SavedUnitHash>);

impl From<&CargoRestoreRequest2> for CargoRestoreRequest2 {
    fn from(req: &CargoRestoreRequest2) -> Self {
        req.clone()
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize, From, AsRef)]
#[non_exhaustive]
pub struct CargoRestoreResponse2(HashMap<SavedUnitHash, UnitSavePlan>);

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
