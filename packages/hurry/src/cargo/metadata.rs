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

use super::workspace::ProfileDir;
use crate::{Locked, fs};

/// Rust compiler metadata for cache key generation.
///
/// Contains platform-specific compiler information needed to generate
/// cache keys that are valid only for the current compilation target.
/// This ensures cached artifacts are not incorrectly shared between
/// different platforms or compiler configurations.
///
/// Currently only captures the LLVM target triple, but could be extended
/// to include compiler version, feature flags, or other compilation options
/// that affect output compatibility.
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
/// Cargo generates `.d` files in the `deps/` directory that follow a
/// makefile-like format: `output: input1 input2 ...`.
/// These files list the artifacts (`.rlib`, `.rmeta`, `.d` files) produced
/// when compiling a dependency.
///
/// This parser extracts the output paths from these files, which are then used
/// by the caching system to identify which files need to be backed up and
/// restored for each dependency.
#[derive(Debug)]
pub struct Dotd {
    /// Recorded output paths, relative to the profile root.
    pub outputs: Vec<RelativePathBuf>,
}

impl Dotd {
    /// Parse a `.d` file and extract output artifact paths.
    ///
    /// Reads the dependency file at the given path (relative to profile root),
    /// parses each line for the `output:` format, and filters for relevant
    /// file extensions. All returned paths are relative to the profile root.
    ///
    /// ## Contract
    /// - Requires a locked [`ProfileDir`] to access the file system
    /// - Target path must be relative to the profile root
    /// - Only extracts outputs with extensions: `.d`, `.rlib`, `.rmeta`
    /// - Returns paths relative to the profile root for cache consistency
    #[instrument(name = "Dotd::from_file")]
    pub async fn from_file(
        profile: &ProfileDir<'_, Locked>,
        target: &RelativePath,
    ) -> Result<Self> {
        const DEP_EXTS: [&str; 4] = [".d", ".rlib", ".rmeta", ".so"];
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
