use std::fmt::Debug;

use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, bail, eyre},
};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tap::TapFallible;
use tracing::{instrument, trace};

use super::workspace::ProfileDir;
use crate::{
    Locked,
    ext::{then_context, then_with_context},
    fs::{self, DEFAULT_CONCURRENCY},
    path::{AbsDirPath, AbsFilePath, JoinWith, RelFilePath, RelativeTo},
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
    pub async fn from_argv(workspace_root: &AbsDirPath, _argv: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new("rustc");

        // Bypasses the check that disallows using unstable commands on stable.
        cmd.env("RUSTC_BOOTSTRAP", "1");
        cmd.args(["-Z", "unstable-options", "--print", "target-spec-json"]);
        cmd.current_dir(workspace_root.as_std_path());
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

/// A parsed "dep-info" file.
///
/// `rustc` generates "dep-info" files in the `deps/` directory that follow a
/// makefile-like format: `output: input1 input2 ...`. It also supports
/// comments and blank lines, which we also retain.
///
/// On disk, each output and input in the file is recorded using an
/// absolute path, but this isn't portable across projects or machines.
/// For this reason, the parsed representation here uses relative paths.
///
/// ## Example
///
/// ```not_rust
/// /Users/jess/projects/hurry-tests/target/debug/deps/humantime-1c46d64671e0aaa7.d: /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/lib.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/date.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/duration.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/wrapper.rs
///
/// /Users/jess/projects/hurry-tests/target/debug/deps/libhumantime-1c46d64671e0aaa7.rlib: /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/lib.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/date.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/duration.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/wrapper.rs
///
/// /Users/jess/projects/hurry-tests/target/debug/deps/libhumantime-1c46d64671e0aaa7.rmeta: /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/lib.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/date.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/duration.rs /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/wrapper.rs
///
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/lib.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/date.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/duration.rs:
/// /Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/humantime-2.2.0/src/wrapper.rs:
/// ```
///
/// ## Future work/TODO
///
/// Today this only handles the `RustcDepInfo` representation[^1];
/// if we end up needing to parse the Cargo's `EncodedDepInfo`[^2] we should
/// either disambiguate this type or make it handle both.
///
/// [^1]: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/fingerprint/dep_info/struct.RustcDepInfo.html
/// [^2]: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/fingerprint/dep_info/struct.EncodedDepInfo.html
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct DepInfo(Vec<DepInfoLine>);

impl DepInfo {
    /// Parse a "dep-info" file and extract output artifact paths.
    ///
    /// Reads the dependency file at the given path (relative to profile root),
    /// parses each line for the `output:` format, and filters for relevant
    /// file extensions. All returned paths are relative to the profile root.
    #[instrument(name = "DepInfo::from_file")]
    pub async fn from_file(profile: &ProfileDir<'_, Locked>, dotd: &AbsFilePath) -> Result<Self> {
        let content = fs::read_buffered_utf8(dotd)
            .await
            .context("read file")?
            .ok_or_eyre("file does not exist")?;
        let lines = stream::iter(content.lines())
            .then(|line| {
                DepInfoLine::parse(profile, &line)
                    .then_with_context(move || format!("parse line: {line:?}"))
            })
            .try_collect::<Vec<_>>()
            .await?;

        trace!(?dotd, ?content, ?lines, "parsed DepInfo file");
        Ok(Self(lines))
    }

    /// Reconstruct the "dep-info" file in the context of the profile directory.
    #[instrument(name = "DepInfo::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        self.0
            .iter()
            .map(|line| line.reconstruct(profile))
            .join("\n")
    }

    /// Iterate over the lines in the file.
    #[instrument(name = "DepInfo::lines")]
    pub fn lines(&self) -> impl Iterator<Item = &DepInfoLine> {
        self.0.iter()
    }

    /// Iterate over builds parsed in the file.
    #[instrument(name = "DepInfo::builds")]
    pub fn builds(&self) -> impl Iterator<Item = (&QualifiedPath, &[QualifiedPath])> {
        self.0.iter().filter_map(|line| match line {
            DepInfoLine::Build(output, inputs) => Some((output, inputs.as_slice())),
            _ => None,
        })
    }

    /// Iterate over build outputs parsed in the file.
    #[instrument(name = "DepInfo::build_outputs")]
    pub fn build_outputs(&self) -> impl Iterator<Item = &QualifiedPath> {
        self.0.iter().filter_map(|line| match line {
            DepInfoLine::Build(output, _) => Some(output),
            _ => None,
        })
    }
}

/// A single line inside a ["dep-info" file](DepInfo).
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum DepInfoLine {
    /// An empty line.
    Space,

    /// A commented line with the inner text following the comment.
    Comment(String),

    /// An output and the set of its inputs.
    ///
    /// Note that every input is _also_ an output, just with an empty
    /// set of inputs.
    /// Outputs are usually only relative to $CARGO_HOME in this case.
    Build(QualifiedPath, Vec<QualifiedPath>),
}

impl DepInfoLine {
    /// Parse the line in a "dep-info" file.
    //
    // TODO: Handle spaces in the paths; rustc uses `\` to escape them[^1].
    // TODO: Handle optional `checksum` comments[^2].
    // TODO: Find other edge cases according to the type[^3] and parser[^4].
    //
    // [^1]: https://doc.rust-lang.org/nightly/nightly-rustc/src/cargo/core/compiler/fingerprint/dep_info.rs.html#406-418
    // [^2]: https://doc.rust-lang.org/nightly/nightly-rustc/src/cargo/core/compiler/fingerprint/dep_info.rs.html#419-435
    // [^3]: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/fingerprint/dep_info/struct.RustcDepInfo.html
    // [^4]: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/fingerprint/dep_info/fn.parse_rustc_dep_info.html
    #[instrument(name = "DepInfoLine::parse")]
    pub async fn parse(profile: &ProfileDir<'_, Locked>, line: &str) -> Result<Self> {
        Ok(if line.is_empty() {
            Self::Space
        } else if let Some(comment) = line.strip_prefix('#') {
            Self::Comment(comment.to_string())
        } else if let Some(output) = line.strip_suffix(':') {
            let output = QualifiedPath::parse(profile, output)
                .then_with_context(move || format!("parse output path: {output:?}"))
                .await?;
            Self::Build(output, Vec::new())
        } else {
            let Some((output, inputs)) = line.split_once(": ") else {
                bail!("no output/input separator");
            };

            let output = QualifiedPath::parse(profile, output)
                .then_with_context(move || format!("parse output path: {output:?}"));
            let inputs = stream::iter(inputs.trim().split_whitespace())
                .map(|input| {
                    QualifiedPath::parse(profile, input)
                        .then_with_context(move || format!("parse input path: {input:?}"))
                })
                .buffer_unordered(DEFAULT_CONCURRENCY)
                .try_collect::<Vec<_>>()
                .then_context("parse input paths");
            let (output, inputs) = tokio::try_join!(output, inputs)?;
            Self::Build(output, inputs)
        })
    }

    #[instrument(name = "DepInfoLine::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        match self {
            Self::Build(output, inputs) => {
                let output = output.reconstruct(profile);
                let inputs = inputs
                    .iter()
                    .map(|input| input.reconstruct(profile))
                    .join(" ");
                format!("{output}: {inputs}")
            }
            DepInfoLine::Space => String::new(),
            DepInfoLine::Comment(comment) => format!("#{comment}"),
        }
    }
}

/// A "qualified" path inside a Cargo project.
///
/// Paths in some files, such as "dep-info" files or build script outputs, are
/// sometimes written using absolute paths. However `hurry` wants to know what
/// these paths are relative to so that it can back up and restore paths in
/// different workspaces and machines. This type supports `hurry` being able to
/// determine what kind of path is being referenced.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum QualifiedPath {
    /// The path is "natively" relative without a root prior to `hurry` making
    /// it relative. Such paths are backed up and restored "as-is".
    ///
    /// Note: Since these paths are natively written as relative paths,
    /// it's not necessarily clear to what file these are referring without more
    /// context (such as the kind of file that contained the path and its
    /// location). If this is ever a problem, we'll probably need to change how
    /// we represent this type- maybe e.g. provide a "computed" path relative to
    /// a known root along with the "native" version of the path.
    Rootless(RelFilePath),

    /// The path is relative to the workspace target profile directory.
    RelativeTargetProfile(RelFilePath),

    /// The path is relative to `$CARGO_HOME` for the user.
    RelativeCargoHome(RelFilePath),
}

impl QualifiedPath {
    #[instrument(name = "QualifiedPath::parse")]
    pub async fn parse(profile: &ProfileDir<'_, Locked>, path: &str) -> Result<Self> {
        Ok(if let Ok(rel) = RelFilePath::try_from(path) {
            if fs::exists(profile.root().join(&rel).as_std_path()).await {
                Self::RelativeTargetProfile(rel)
            } else if fs::exists(profile.workspace.cargo_home.join(&rel).as_std_path()).await {
                Self::RelativeCargoHome(rel)
            } else {
                Self::Rootless(rel)
            }
        } else if let Ok(abs) = AbsFilePath::try_from(path) {
            if let Ok(rel) = abs.relative_to(profile.root()) {
                Self::RelativeTargetProfile(rel)
            } else if let Ok(rel) = abs.relative_to(&profile.workspace.cargo_home) {
                Self::RelativeCargoHome(rel)
            } else {
                bail!("unknown root for absolute path: {abs:?}");
            }
        } else {
            bail!("unknown kind of path: {path:?}")
        })
    }

    #[instrument(name = "QualifiedPath::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        match self {
            QualifiedPath::Rootless(rel) => rel.to_string(),
            QualifiedPath::RelativeTargetProfile(rel) => profile.root().join(rel).to_string(),
            QualifiedPath::RelativeCargoHome(rel) => {
                profile.workspace.cargo_home.join(rel).to_string()
            }
        }
    }
}

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
                BuildScriptOutputLine::parse(profile, &line)
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
