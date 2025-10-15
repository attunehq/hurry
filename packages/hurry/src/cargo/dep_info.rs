use std::fmt::Debug;

use color_eyre::{
    Result,
    eyre::{Context, OptionExt, bail},
};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::{instrument, trace};

use super::workspace::ProfileDir;
use crate::{
    Locked,
    cargo::QualifiedPath,
    ext::{then_context, then_with_context},
    fs::{self, DEFAULT_CONCURRENCY},
    path::{AbsDirPath, AbsFilePath},
};

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
                DepInfoLine::parse(profile, line)
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
        self.reconstruct_raw(profile.root(), &profile.workspace.cargo_home)
    }

    /// Reconstruct the "dep-info" file using owned path data.
    #[instrument(name = "DepInfo::reconstruct_raw")]
    pub fn reconstruct_raw(&self, profile_root: &AbsDirPath, cargo_home: &AbsDirPath) -> String {
        self.0
            .iter()
            .map(|line| line.reconstruct_raw(profile_root, cargo_home))
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
            let output = QualifiedPath::parse_string(profile, output)
                .then_with_context(move || format!("parse output path: {output:?}"))
                .await?;
            Self::Build(output, Vec::new())
        } else {
            let Some((output, inputs)) = line.split_once(": ") else {
                bail!("no output/input separator");
            };

            let output = QualifiedPath::parse_string(profile, output)
                .then_with_context(move || format!("parse output path: {output:?}"));
            let inputs = stream::iter(inputs.split_whitespace())
                .map(|input| {
                    QualifiedPath::parse_string(profile, input)
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
        self.reconstruct_raw(profile.root(), &profile.workspace.cargo_home)
    }

    #[instrument(name = "DepInfoLine::reconstruct_raw")]
    pub fn reconstruct_raw(&self, profile_root: &AbsDirPath, cargo_home: &AbsDirPath) -> String {
        match self {
            Self::Build(output, inputs) => {
                let output = output.reconstruct_raw_string(profile_root, cargo_home);
                let inputs = inputs
                    .iter()
                    .map(|input| input.reconstruct_raw_string(profile_root, cargo_home))
                    .join(" ");
                format!("{output}: {inputs}")
            }
            DepInfoLine::Space => String::new(),
            DepInfoLine::Comment(comment) => format!("#{comment}"),
        }
    }
}
