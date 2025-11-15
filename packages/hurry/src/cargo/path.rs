use std::fmt::Debug;

use color_eyre::{Result, eyre::bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    cargo::Workspace,
    fs,
    path::{AbsFilePath, GenericPath, JoinWith as _, RelFilePath, RelativeTo as _},
};

/// A "qualified" path inside a Cargo project.
///
/// Semantically relative paths in some files (e.g. dep-info files, build script
/// outputs, etc.) are sometimes written as resolved absolute paths. However,
/// `hurry` needs to recognize that these paths are relative so it can rewrite
/// them when restoring artifacts to different machines with different paths.
/// This type implements path parsing and rewriting.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum QualifiedPath {
    /// The path is originally written as relative. Such paths are backed up and
    /// restored "as-is".
    Rootless(RelFilePath),

    /// The absolute path is relative to the workspace target profile directory.
    RelativeTargetProfile(RelFilePath),

    /// The absolute path is relative to `$CARGO_HOME` for the user.
    RelativeCargoHome(RelFilePath),

    /// The absolute path is not relative to any known root.
    ///
    /// In practice, these are paths to SDK headers, system libraries, etc.
    /// items that are at known paths on machines. Crates semantically should
    /// not be referencing absolute paths without also emitting Cargo directives
    /// to invalidate builds when the files at those paths change (e.g. see how
    /// the openssl build script discovers the system SSL library[^1]).
    ///
    /// We handle these paths by handling build script output directives.
    ///
    /// In the future, we'll enumerate more roots (e.g. macOS SDK, Homebrew) and
    /// add specific handling if needed.
    ///
    /// [^1]: https://github.com/rust-openssl/rust-openssl/blob/09b90d036ec5341deefb7fce86748e176379d01a/openssl-sys/build/find_normal.rs#L72
    Absolute(AbsFilePath),
}

impl QualifiedPath {
    #[instrument(name = "QualifiedPath::parse_string")]
    pub async fn parse_string(ws: &Workspace, path: &str) -> Result<Self> {
        Self::parse(ws, &GenericPath::try_from(path)?).await
    }

    #[instrument(name = "QualifiedPath::parse")]
    pub async fn parse(ws: &Workspace, path: &GenericPath) -> Result<Self> {
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(ws.profile_dir.join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(ws.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(&ws.profile_dir) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = abs.relative_to(&ws.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Absolute(abs)
            }
        } else {
            bail!("unknown kind of path: {path:?}")
        })
    }

    #[instrument(name = "QualifiedPath::reconstruct_string")]
    pub fn reconstruct_string(&self, ws: &Workspace) -> String {
        Self::reconstruct(self, ws).to_string()
    }

    #[instrument(name = "QualifiedPath::reconstruct")]
    pub fn reconstruct(&self, ws: &Workspace) -> GenericPath {
        match self {
            QualifiedPath::Rootless(rel) => rel.into(),
            QualifiedPath::RelativeTargetProfile(rel) => ws.profile_dir.join(rel).into(),
            QualifiedPath::RelativeCargoHome(rel) => ws.cargo_home.join(rel).into(),
            QualifiedPath::Absolute(abs) => abs.into(),
        }
    }
}
