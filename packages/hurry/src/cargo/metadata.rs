use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use cargo_metadata::camino::Utf8Path;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, bail, eyre},
};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use relative_path::{PathExt, RelativePathBuf};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{instrument, trace};

use super::workspace::ProfileDir;
use crate::{
    Locked,
    ext::{then_context, then_with_context},
    fs::{self, DEFAULT_CONCURRENCY},
};

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
/// makefile-like format: `output: input1 input2 ...`. It also supports
/// comments and blank lines, which we also retain.
///
/// On disk, each output and input in the file is recorded using an
/// absolute path, but this isn't portable across projects or machines.
/// For this reason, the parsed representation here uses relative paths.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct Dotd(Vec<DotdLine>);

impl Dotd {
    /// Parse a `.d` file and extract output artifact paths.
    ///
    /// Reads the dependency file at the given path (relative to profile root),
    /// parses each line for the `output:` format, and filters for relevant
    /// file extensions. All returned paths are relative to the profile root.
    #[instrument(name = "Dotd::from_file")]
    pub async fn from_file(
        profile: &ProfileDir<'_, Locked>,
        dotd: impl AsRef<Path> + Debug,
    ) -> Result<Self> {
        let dotd = dotd.as_ref();
        let content = fs::read_buffered_utf8(dotd)
            .await
            .context("read file")?
            .ok_or_eyre("file does not exist")?;
        let lines = stream::iter(content.lines())
            .then(|line| {
                DotdLine::parse(profile, &line)
                    .then_with_context(move || format!("parse line: {line:?}"))
            })
            .try_collect::<Vec<_>>()
            .await?;

        trace!(?dotd, ?content, ?lines, "parsed .d file");
        Ok(Self(lines))
    }

    /// Reconstruct the `.d` file at the provided path.
    #[instrument(name = "Dotd::reconstruct")]
    pub async fn reconstruct(
        &self,
        profile: &ProfileDir<'_, Locked>,
        dotd: impl AsRef<Path> + Debug,
    ) -> Result<()> {
        let dotd = dotd.as_ref();
        let content = self
            .0
            .iter()
            .map(|line| line.reconstruct(profile))
            .join("\n");
        fs::write(dotd, content).await
    }

    /// Iterate over the lines in the file.
    #[instrument(name = "Dotd::lines")]
    pub fn lines(&self) -> impl Iterator<Item = &DotdLine> {
        self.0.iter()
    }

    /// Iterate over builds parsed in the file.
    #[instrument(name = "Dotd::builds")]
    pub fn builds(&self) -> impl Iterator<Item = (&RelativePathBuf, &[DotdDependencyPath])> {
        self.0.iter().filter_map(|line| match line {
            DotdLine::Build(output, inputs) => Some((output, inputs.as_slice())),
            _ => None,
        })
    }

    /// Iterate over build outputs parsed in the file.
    #[instrument(name = "Dotd::build_outputs")]
    pub fn build_outputs(&self) -> impl Iterator<Item = &RelativePathBuf> {
        self.0.iter().filter_map(|line| match line {
            DotdLine::Build(output, _) => Some(output),
            _ => None,
        })
    }
}

/// A single line inside a `.d` file.
/// Refer to [`Dotd`] for more details.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum DotdLine {
    /// An empty line.
    Space,

    /// A commented line with the inner text following the comment.
    Comment(String),

    /// An output and the set of its inputs.
    /// The output is relative to the profile root directory.
    Build(RelativePathBuf, Vec<DotdDependencyPath>),
}

impl DotdLine {
    /// Parse the line in a `.d` file.
    //
    // TODO: We almost definitely need to handle spaces in the paths.
    #[instrument(name = "DotdLine::parse")]
    async fn parse(profile: &ProfileDir<'_, Locked>, line: &str) -> Result<Self> {
        Ok(if line.is_empty() {
            Self::Space
        } else if let Some(comment) = line.strip_prefix('#') {
            Self::Comment(comment.to_string())
        } else if let Some(output) = line.strip_suffix(':') {
            let output = PathBuf::from(output)
                .relative_to(profile.root())
                .with_context(|| format!("make {output:?} relative to {profile:?}"))?;
            Self::Build(output, Vec::new())
        } else {
            let Some((output, inputs)) = line.split_once(": ") else {
                bail!("no output/input separator");
            };

            let output = PathBuf::from(output)
                .relative_to(profile.root())
                .with_context(|| format!("make {output:?} relative to {profile:?}"))?;
            let inputs = stream::iter(inputs.trim().split_whitespace())
                .map(|input| {
                    DotdDependencyPath::parse(profile, input)
                        .then_with_context(move || format!("parse input path: {input:?}"))
                })
                .buffer_unordered(DEFAULT_CONCURRENCY)
                .try_collect::<Vec<_>>()
                .then_context("parse input paths")
                .await?;
            Self::Build(output, inputs)
        })
    }

    #[instrument(name = "DotdLine::reconstruct")]
    fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        match self {
            Self::Build(output, inputs) => {
                let output = output.to_path(profile.root()).to_string_lossy().to_string();
                let inputs = inputs
                    .iter()
                    .map(|input| input.reconstruct(profile))
                    .join(" ");
                format!("{output}: {inputs}")
            }
            DotdLine::Space => String::new(),
            DotdLine::Comment(comment) => format!("#{comment}"),
        }
    }
}

/// A dependency path specified in a `.d` file.
///
/// Dependencies specified in `.d` files can reference files either inside the
/// current project, or in the Cargo registry cache on the local machine.
/// This type differentiates between these options.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum DotdDependencyPath {
    /// The path is relative to the workspace target profile directory.
    RelativeTargetProfile(RelativePathBuf),

    /// The path is relative to `$CARGO_HOME` for the user.
    RelativeCargoHome(RelativePathBuf),
}

impl DotdDependencyPath {
    #[instrument(name = "DotdPathBuf::parse")]
    async fn parse(profile: &ProfileDir<'_, Locked>, path: &str) -> Result<Self> {
        let path = PathBuf::from(path);
        Ok(if let Ok(rel) = RelativePathBuf::from_path(&path) {
            if fs::exists(rel.to_path(profile.root())).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(rel.to_path(&profile.workspace.cargo_home)).await {
                Self::RelativeCargoHome(rel)
            } else {
                bail!("unknown root for relative path: {rel:?}");
            }
        } else {
            if let Ok(rel) = path.relative_to(profile.root()) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = path.relative_to(&profile.workspace.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                bail!("unknown root for absolute path: {path:?}");
            }
        })
    }

    #[instrument(name = "DotdPathBuf::to_path")]
    fn to_path(&self, profile: &ProfileDir<'_, Locked>) -> PathBuf {
        match self {
            DotdDependencyPath::RelativeTargetProfile(rel) => rel.to_path(profile.root()),
            DotdDependencyPath::RelativeCargoHome(rel) => {
                rel.to_path(&profile.workspace.cargo_home)
            }
        }
    }

    #[instrument(name = "DotdPathBuf::reconstruct")]
    fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        self.to_path(profile).to_string_lossy().to_string()
    }
}
