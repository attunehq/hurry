//! Cargo cache API types.

use std::collections::{HashMap, HashSet};

use bon::Builder;
use derive_more::{AsRef, From};
use serde::{Deserialize, Serialize};

use crate::courier::v1::{SavedFile, SavedUnitHash, UnitSavePlan};

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
