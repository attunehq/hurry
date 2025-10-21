//! CAS-specific API types.

use std::collections::BTreeSet;

use bon::Builder;
use serde::{Deserialize, Serialize};

use super::Key;

/// Response from bulk CAS write operation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Builder)]
#[non_exhaustive]
pub struct CasBulkWriteResponse {
    #[builder(with = |i: impl IntoIterator<Item = impl Into<Key>>| i.into_iter().map(Into::into).collect())]
    pub written: BTreeSet<Key>,

    #[builder(with = |i: impl IntoIterator<Item = impl Into<Key>>| i.into_iter().map(Into::into).collect())]
    pub skipped: BTreeSet<Key>,

    #[builder(with = |i: impl IntoIterator<Item = impl Into<BulkWriteKeyError>>| i.into_iter().map(Into::into).collect())]
    pub errors: BTreeSet<BulkWriteKeyError>,
}

impl From<&CasBulkWriteResponse> for CasBulkWriteResponse {
    fn from(response: &CasBulkWriteResponse) -> Self {
        response.clone()
    }
}

/// Error for a specific key during bulk write operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize, Builder)]
#[non_exhaustive]
pub struct BulkWriteKeyError {
    #[builder(into)]
    pub key: Key,

    #[builder(into)]
    pub error: String,
}

impl From<&BulkWriteKeyError> for BulkWriteKeyError {
    fn from(err: &BulkWriteKeyError) -> Self {
        err.clone()
    }
}

/// Request body for bulk CAS read operation.
#[derive(Clone, Eq, PartialEq, Debug, Deserialize, Serialize, Builder)]
#[non_exhaustive]
pub struct CasBulkReadRequest {
    #[builder(with = |i: impl IntoIterator<Item = impl Into<Key>>| i.into_iter().map(Into::into).collect())]
    pub keys: Vec<Key>,
}

impl From<&CasBulkReadRequest> for CasBulkReadRequest {
    fn from(request: &CasBulkReadRequest) -> Self {
        request.clone()
    }
}
