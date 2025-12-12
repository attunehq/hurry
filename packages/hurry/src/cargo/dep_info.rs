use std::fmt::Debug;

use color_eyre::{
    Result,
    eyre::{Context, OptionExt, bail},
};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tap::Pipe;
use tracing::{instrument, trace};

use crate::{
    cargo::{QualifiedPath, RustcTarget, UnitPlanInfo, Workspace},
    ext::{then_context, then_with_context},
    fs::{self, DEFAULT_CONCURRENCY},
    path::AbsFilePath,
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
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
pub struct DepInfo(Vec<DepInfoLine>);

impl DepInfo {
    /// Parse a "dep-info" file and extract output artifact paths.
    ///
    /// Reads the dependency file at the given path (relative to profile root),
    /// parses each line for the `output:` format, and filters for relevant
    /// file extensions. All returned paths are relative to the profile root.
    #[instrument(name = "DepInfo::from_file")]
    pub async fn from_file(
        ws: &Workspace,
        target: &RustcTarget,
        dotd: &AbsFilePath,
    ) -> Result<Self> {
        let content = fs::read_buffered_utf8(dotd)
            .await
            .context("read file")?
            .ok_or_eyre("file does not exist")?;

        let lines = escaped_lines(&content)
            .pipe(stream::iter)
            .then(|line| async move {
                DepInfoLine::parse(ws, target, &line)
                    .await
                    .with_context(|| format!("parse line: {line:?}"))
            })
            .try_collect::<Vec<_>>()
            .await?;

        trace!(?dotd, ?content, ?lines, "parsed DepInfo file");
        Ok(Self(lines))
    }

    /// Reconstruct the "dep-info" file in the context of the profile directory.
    #[instrument(name = "DepInfo::reconstruct", skip_all)]
    pub fn reconstruct(self, ws: &Workspace, unit_info: &UnitPlanInfo) -> String {
        self.0
            .into_iter()
            .map(|line| line.reconstruct(ws, unit_info))
            .collect::<Vec<_>>()
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
#[derive(Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize)]
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
    pub async fn parse(ws: &Workspace, target: &RustcTarget, line: &str) -> Result<Self> {
        Ok(if line.is_empty() {
            Self::Space
        } else if let Some(comment) = line.strip_prefix('#') {
            Self::Comment(comment.to_string())
        } else if let Some(output) = line.strip_suffix(':') {
            let output = QualifiedPath::parse_string(ws, target, output)
                .then_with_context(move || format!("parse output path: {output:?}"))
                .await?;
            Self::Build(output, Vec::new())
        } else {
            let Some((output, inputs)) = line.split_once(": ") else {
                bail!("no output/input separator");
            };

            let output = QualifiedPath::parse_string(ws, target, output)
                .then_with_context(move || format!("parse output path: {output:?}"));
            let inputs = inputs
                .split_whitespace()
                .map(String::from)
                .collect_vec()
                .pipe(stream::iter)
                .map(|input| async move {
                    QualifiedPath::parse_string(ws, target, &input)
                        .await
                        .with_context(move || format!("parse input path: {input:?}"))
                })
                .buffer_unordered(DEFAULT_CONCURRENCY)
                .try_collect::<Vec<_>>()
                .then_context("parse input paths");
            let (output, inputs) = tokio::try_join!(output, inputs)?;
            Self::Build(output, inputs)
        })
    }

    #[instrument(name = "DepInfoLine::reconstruct", skip_all)]
    pub fn reconstruct(self, ws: &Workspace, unit_info: &UnitPlanInfo) -> String {
        match self {
            Self::Build(output, inputs) => {
                let output = output.reconstruct(ws, unit_info).to_string();
                let inputs = inputs
                    .into_iter()
                    .map(|input| input.reconstruct(ws, unit_info).to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{output}: {inputs}")
            }
            DepInfoLine::Space => String::new(),
            DepInfoLine::Comment(comment) => format!("#{comment}"),
        }
    }
}

/// Split the content into lines, except that backslashes escape line breaks.
fn escaped_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        match line.strip_suffix('\\') {
            Some(stripped) => {
                current.push_str(stripped);
            }
            None => {
                current.push_str(line);
                result.push(current);
                current = String::new();
            }
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use simple_test_case::test_case;

    use super::*;

    #[test_case(
        "line1 \\\nline2 \\\nline3\nline4",
        vec![String::from("line1 line2 line3"), String::from("line4")];
        "continuation_lines"
    )]
    #[test_case(
        "line1\nline2\nline3",
        vec![String::from("line1"), String::from("line2"), String::from("line3")];
        "no_continuations"
    )]
    #[test_case(
        "",
        vec![];
        "empty_input"
    )]
    #[test_case(
        "line1 \\\nline2 \\",
        vec![String::from("line1 line2 ")];
        "trailing_backslash"
    )]
    #[test_case(
        "  line1  \\\n  line2  \\\n  line3  ",
        vec![String::from("  line1    line2    line3  ")];
        "preserves_whitespace"
    )]
    #[test]
    fn parses_escaped_lines(input: &str, expected: Vec<String>) {
        let result = escaped_lines(input);
        pretty_assert_eq!(result, expected);
    }
}
