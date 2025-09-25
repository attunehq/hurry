use std::fmt::Debug;

use color_eyre::{
    Result,
    eyre::{Context, OptionExt, eyre},
};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tap::TapFallible;
use tracing::{instrument, trace};

use super::workspace::ProfileDir;
use crate::{Locked, cargo::QualifiedPath, ext::then_with_context, fs, path::AbsFilePath};

/// Represents a "root output" file, used for build scripts.
///
/// This file contains the fully qualified path to `out`, which is the directory
/// where script can output files (provided to the script as $OUT_DIR).
///
/// Example:
/// ```not_rust
/// /Users/jess/scratch/example/target/debug/build/rustls-5590c033895e7e9a/out
/// ```
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct RootOutput(QualifiedPath);

impl RootOutput {
    /// Parse a "root output" file.
    #[instrument(name = "RootOutput::from_file")]
    pub async fn from_file(profile: &ProfileDir<'_, Locked>, file: &AbsFilePath) -> Result<Self> {
        let content = fs::read_buffered_utf8(file)
            .await
            .context("read file")?
            .ok_or_eyre("file does not exist")?;
        let line = content
            .lines()
            .exactly_one()
            .map_err(|_| eyre!("RootOutput file has more than one line: {content:?}"))?;
        QualifiedPath::parse(profile, line)
            .await
            .context("parse file")
            .map(Self)
            .tap_ok(|parsed| trace!(?file, ?content, ?parsed, "parsed RootOutput file"))
    }

    /// Reconstruct the file in the context of the profile directory.
    #[instrument(name = "RootOutput::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        format!("{}", self.0.reconstruct(profile))
    }
}

/// Parsed representation of the output of a build script when it was executed.
///
/// These are correct to rewrite because paths in this output will almost
/// definitely be referencing either something local or something in
/// `$CARGO_HOME`.
///
/// Example output taken from an actual project:
/// ```not_rust
/// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
/// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
/// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
/// OUT_DIR = Some(/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out)
/// cargo:rustc-link-search=native=/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out
/// cargo:root=/Users/jess/scratch/example/target/debug/build/zstd-sys-eb89796c05cc5c90/out
/// cargo:include=/Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/zstd-sys-2.0.15+zstd.1.5.7/zstd/lib
/// ```
///
/// Reference: https://doc.rust-lang.org/cargo/reference/build-scripts.html
#[derive(Clone, Eq, PartialEq, Debug, Deserialize, Serialize)]
pub struct BuildScriptOutput(Vec<BuildScriptOutputLine>);

impl BuildScriptOutput {
    /// Parse a build script output file.
    #[instrument(name = "BuildScriptOutput::from_file")]
    pub async fn from_file(profile: &ProfileDir<'_, Locked>, file: &AbsFilePath) -> Result<Self> {
        let content = fs::read_buffered_utf8(file)
            .await
            .context("read file")?
            .ok_or_eyre("file does not exist")?;
        let lines = stream::iter(content.lines())
            .then(|line| {
                BuildScriptOutputLine::parse(profile, line)
                    .then_with_context(move || format!("parse line: {line:?}"))
            })
            .try_collect::<Vec<_>>()
            .await?;

        trace!(?file, ?content, ?lines, "parsed DepInfo file");
        Ok(Self(lines))
    }

    /// Reconstruct the file in the context of the profile directory.
    #[instrument(name = "BuildScriptOutput::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        self.0
            .iter()
            .map(|line| line.reconstruct(profile))
            .join("\n")
    }
}

/// Build scripts communicate with Cargo by printing to stdout. Cargo will
/// interpret each line that starts with cargo:: as an instruction that will
/// influence compilation of the package. All other lines are ignored.
///
/// `hurry` only cares about parsing some directives; directives it doesn't care
/// about are passed through unchanged as the `Other` variant.
///
/// Reference for possible options according to the Cargo docs:
/// https://doc.rust-lang.org/cargo/reference/build-scripts.html#outputs-of-the-build-script
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub enum BuildScriptOutputLine {
    /// `cargo::rerun-if-changed=PATH`
    RerunIfChanged(QualifiedPath),

    /// All other lines.
    ///
    /// These are intended to be backed up and restored unmodified.
    /// No guarantees are made about these lines: they could be blank
    /// or contain other arbitrary content.
    Other(String),
    //
    // Commented for now until we have the concept of multiple cache keys.
    // Once we have those we'll need to re-add this and then make it influence
    // the cache key:
    // https://attunehq-workspace.slack.com/archives/C08ALDYV85T/p1757723284399379
    //
    // /// `cargo::rustc-link-search=[KIND=]PATH`
    // RustcLinkSearch(Option<String>, QualifiedPath),
}

impl BuildScriptOutputLine {
    const RERUN_IF_CHANGED: &str = "cargo:rerun-if-changed";
    // const RUSTC_LINK_SEARCH: &str = "cargo:rustc-link-search";

    /// Parse a line of the build script file.
    #[instrument(name = "BuildScriptOutputLine::parse")]
    pub async fn parse(profile: &ProfileDir<'_, Locked>, line: &str) -> Result<Self> {
        if let Some((key, value)) = line.split_once('=') {
            match key {
                Self::RERUN_IF_CHANGED => {
                    let path = QualifiedPath::parse(profile, value).await?;
                    Ok(Self::RerunIfChanged(path))
                }
                _ => Ok(Self::Other(line.to_string())),
                //
                // Commented for now, context:
                // https://attunehq-workspace.slack.com/archives/C08ALDYV85T/p1757723284399379
                //
                // Self::RUSTC_LINK_SEARCH => {
                //     if let Some((kind, path)) = value.split_once('=') {
                //         let path = QualifiedPath::parse(profile, path).await?;
                //         let kind = Some(kind.to_string());
                //         Ok(Self::RustcLinkSearch(kind, path))
                //     } else {
                //         let path = QualifiedPath::parse(profile, value).await?;
                //         Ok(Self::RustcLinkSearch(None, path))
                //     }
                // }
            }
        } else {
            Ok(Self::Other(line.to_string()))
        }
    }

    /// Reconstruct the line in the current context.
    #[instrument(name = "BuildScriptOutputLine::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        match self {
            BuildScriptOutputLine::RerunIfChanged(path) => {
                format!("{}={}", Self::RERUN_IF_CHANGED, path.reconstruct(profile))
            }
            BuildScriptOutputLine::Other(s) => s.to_string(),
            //
            // Commented for now, context:
            // https://attunehq-workspace.slack.com/archives/C08ALDYV85T/p1757723284399379
            //
            // BuildScriptOutputLine::RustcLinkSearch(Some(kind), path) => format!(
            //     "{}={}={}",
            //     Self::RUSTC_LINK_SEARCH,
            //     kind,
            //     path.reconstruct(profile)
            // ),
            // BuildScriptOutputLine::RustcLinkSearch(None, path) => {
            //     format!("{}={}", Self::RUSTC_LINK_SEARCH, path.reconstruct(profile))
            // }
        }
    }
}
