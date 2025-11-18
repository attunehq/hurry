//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::{AsRef, From};
use serde::{Deserialize, Serialize};

use crate::courier::v1::{SavedFile, SavedUnitHash, UnitSavePlan};

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
