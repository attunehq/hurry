//! CAS-specific API types.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::Key;

/// Response from bulk CAS write operation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CasBulkWriteResponse {
    pub written: BTreeSet<Key>,
    pub skipped: BTreeSet<Key>,
    pub errors: BTreeSet<BulkWriteKeyError>,
}

/// Error for a specific key during bulk write operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub struct BulkWriteKeyError {
    pub key: Key,
    pub error: String,
}

/// Request body for bulk CAS read operation.
#[derive(Debug, Serialize)]
pub struct CasBulkReadRequest {
    pub keys: Vec<Key>,
}
