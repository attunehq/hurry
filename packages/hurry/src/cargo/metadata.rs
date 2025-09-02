use std::str::FromStr;

use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, eyre},
};
use relative_path::{RelativePath, RelativePathBuf};
use serde::Deserialize;
use tap::TapFallible;
use tracing::{instrument, trace};

use crate::{Locked, fs};
use super::workspace::ProfileDir;

/// Rust's compiler options for the current platform.
///
/// This isn't the _full_ set of options,
/// just what we need for caching.
//
// TODO: Support users cross compiling; probably need to parse argv?
// TODO: Determine minimum compiler version.
// TODO: Is there a better way to get this?
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize)]
pub struct RustcMetadata {
    /// The LLVM target triple.
    #[serde(rename = "llvm-target")]
    pub llvm_target: String,
}

impl RustcMetadata {
    /// Get platform metadata from the current compiler.
    #[instrument(name = "RustcMetadata::from_argv")]
    pub async fn from_argv(workspace_root: &Utf8Path, _argv: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new("rustc");

        // Bypasses the check that disallows using unstable commands on stable.
        cmd.env("RUSTC_BOOTSTRAP", "1");
        cmd.args(["-Z", "unstable-options", "--print", "target-spec-json"]);
        cmd.current_dir(workspace_root);
        let output = cmd.output().await.context("run rustc")?;
        if !output.status.success() {
            return Err(eyre!("invoke rustc"))
                .with_section(|| {
                    String::from_utf8_lossy(&output.stdout)
                        .to_string()
                        .header("Stdout:")
                })
                .with_section(|| {
                    String::from_utf8_lossy(&output.stderr)
                        .to_string()
                        .header("Stderr:")
                });
        }

        serde_json::from_slice::<RustcMetadata>(&output.stdout)
            .context("parse rustc output")
            .with_section(|| {
                String::from_utf8_lossy(&output.stdout)
                    .to_string()
                    .header("Rustc Output:")
            })
    }
}

/// A parsed Cargo .d file.
///
/// `.d` files are structured a little like makefiles, where each output
/// is on its own line followed by a colon followed by the inputs.
#[derive(Debug)]
pub struct Dotd {
    /// Recorded output paths, relative to the profile root.
    pub outputs: Vec<RelativePathBuf>,
}

impl Dotd {
    /// Construct an instance by parsing the file.
    #[instrument(name = "Dotd::from_file")]
    pub async fn from_file(
        profile: &ProfileDir<'_, Locked>,
        target: &RelativePath,
    ) -> Result<Self> {
        const DEP_EXTS: [&str; 3] = [".d", ".rlib", ".rmeta"];
        let profile_root = profile.root();
        let outputs = fs::read_buffered_utf8(target.to_path(&profile_root))
            .await
            .with_context(|| format!("read .d file: {target:?}"))?
            .ok_or_eyre("file does not exist")?
            .lines()
            .filter_map(|line| {
                let (output, _) = line.split_once(':')?;
                if DEP_EXTS.iter().any(|ext| output.ends_with(ext)) {
                    trace!(?line, ?output, "read .d line");
                    Utf8PathBuf::from_str(output)
                        .tap_err(|error| trace!(?line, ?output, ?error, "not a valid path"))
                        .ok()
                } else {
                    trace!(?line, "skipped .d line");
                    None
                }
            })
            .map(|output| -> Result<RelativePathBuf> {
                output
                    .strip_prefix(&profile_root)
                    .with_context(|| format!("make {output:?} relative to {profile_root:?}"))
                    .and_then(|p| RelativePathBuf::from_path(p).context("read path as utf8"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { outputs })
    }
}