use std::iter::once;

use bon::Builder;
use cargo_metadata::camino::Utf8PathBuf;
use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

mod cmd;
mod workspace;

pub use cmd::*;

use crate::{cargo::workspace::Workspace, hash::Blake3};

/// Invoke a cargo subcommand with the given arguments.
#[instrument(skip_all)]
pub fn invoke(
    workspace: &Workspace,
    subcommand: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<()> {
    let args = args.into_iter().collect::<Vec<_>>();
    let args = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>();

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&workspace.root);
    cmd.args(once(subcommand.as_ref()).chain(args));
    let status = cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .context("could complete cargo execution")?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo exited with status: {status}");
    }
}

/// Records backed up cache artifacts for third party crates.
///
/// A cache record links a dependency (in `Cargo.toml`)
/// to one or more artifacts in the CAS
/// by the dependency's key and the CAS hash.
#[derive(Debug, Serialize, Deserialize, Builder)]
pub struct CacheRecord {
    /// The dependency to which this cache corresponds.
    #[builder(into)]
    pub dependency_key: Blake3,

    /// The artifacts in this record.
    #[builder(default, into)]
    pub artifacts: Vec<CacheRecordArtifact>,
}

/// A recorded cache artifact.
#[derive(Debug, Serialize, Deserialize, Builder)]
pub struct CacheRecordArtifact {
    /// The relative location within the profile folder
    /// to copy the cached artifact when restoring the cache.
    #[builder(into)]
    pub target: Utf8PathBuf,

    /// The hash of the cached artifact.
    /// Used to reference the artifact in the `hurry` CAS.
    #[builder(into)]
    pub hash: Blake3,
}
