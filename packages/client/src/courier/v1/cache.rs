//! Cargo cache API types.

use std::fmt::Display;

use bon::Builder;
use derive_more::Deref;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::Key;

/// Path to an artifact file in the cargo cache.
/// The path is stored as a JSON-encoded string internally, but provides
/// transparent conversion to/from `QualifiedPath` (in the hurry crate).
#[derive(Clone, PartialEq, Eq, Debug, Deref)]
#[deref(forward)]
pub struct ArtifactFilePath(String);

impl ArtifactFilePath {
    /// Create an artifact file path from a JSON-encoded string.
    pub fn new(json: String) -> Self {
        Self(json)
    }

    /// View the path as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ArtifactFilePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ArtifactFilePath {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl Serialize for ArtifactFilePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ArtifactFilePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self)
    }
}

/// An artifact file in the cargo cache.
/// The path is stored as a JSON-encoded string.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct ArtifactFile {
    pub object_key: Key,
    pub mtime_nanos: u128,
    pub executable: bool,
    pub path: ArtifactFilePath,
}

/// Request to save cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct CargoSaveRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
    pub content_hash: String,
    pub artifacts: Vec<ArtifactFile>,
}

/// Request to restore cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Builder)]
#[builder(on(String, into))]
pub struct CargoRestoreRequest {
    pub package_name: String,
    pub package_version: String,
    pub target: String,
    pub library_crate_compilation_unit_hash: String,
    pub build_script_compilation_unit_hash: Option<String>,
    pub build_script_execution_unit_hash: Option<String>,
}

/// Response from restoring cargo cache metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct CargoRestoreResponse {
    pub artifacts: Vec<ArtifactFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_file_path_serialization() {
        let path = ArtifactFilePath::new(String::from(r#"{"path":"lib.rlib"}"#));
        let json = serde_json::to_string(&path).unwrap();
        // The JSON-encoded string gets serialized again by serde_json
        assert_eq!(json, r#""{\"path\":\"lib.rlib\"}""#);

        let deserialized: ArtifactFilePath = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, path);
    }

    #[test]
    fn artifact_file_path_as_str() {
        let path = ArtifactFilePath::new(String::from(r#"{"path":"lib.rlib"}"#));
        assert_eq!(path.as_str(), r#"{"path":"lib.rlib"}"#);
    }
}
