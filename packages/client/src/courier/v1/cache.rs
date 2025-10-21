//! Cargo cache API types.

use bon::Builder;
use serde::{Deserialize, Serialize};

use super::Key;

/// An artifact file in the cargo cache.
/// The path is stored as a JSON-encoded string.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
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

    #[builder(with = |i: impl IntoIterator<Item = impl Into<ArtifactFile>>| i.into_iter().map(Into::into).collect())]
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

impl From<&CargoRestoreRequest> for CargoRestoreRequest {
    fn from(req: &CargoRestoreRequest) -> Self {
        req.clone()
    }
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct CargoRestoreResponse {
    #[builder(with = |i: impl IntoIterator<Item = impl Into<ArtifactFile>>| i.into_iter().map(Into::into).collect())]
    pub artifacts: Vec<ArtifactFile>,
}

impl From<&CargoRestoreResponse> for CargoRestoreResponse {
    fn from(resp: &CargoRestoreResponse) -> Self {
        resp.clone()
    }
}
