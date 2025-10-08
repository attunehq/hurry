use std::fmt::Debug;

use color_eyre::{
    Result,
    eyre::{Context, OptionExt, bail, eyre},
};
use futures::{StreamExt, stream};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tap::TapFallible;
use tracing::{instrument, trace};

use super::workspace::ProfileDir;
use crate::{Locked, cargo::QualifiedPath, fs, path::AbsFilePath};

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
            .then(|line| BuildScriptOutputLine::parse(profile, line))
            .collect::<Vec<_>>()
            .await;

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
/// Reference for possible options according to the Cargo docs:
/// https://doc.rust-lang.org/cargo/reference/build-scripts.html#outputs-of-the-build-script
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub enum BuildScriptOutputLine {
    /// `cargo::rerun-if-changed=PATH`
    RerunIfChanged(QualifiedPath),

    /// `cargo::rerun-if-env-changed=VAR`
    RerunIfEnvChanged(String),

    /// `cargo::rustc-link-arg=FLAG`
    RustcLinkArg(String),

    /// `cargo::rustc-link-lib=LIB`
    RustcLinkLib(String),

    /// `cargo::rustc-link-search=[KIND=]PATH`
    RustcLinkSearch {
        kind: Option<String>,
        path: QualifiedPath,
    },

    /// `cargo::rustc-flags=FLAGS`
    RustcFlags(String),

    /// `cargo::rustc-cfg=KEY[="VALUE"]`
    RustcCfg(String),

    /// `cargo::rustc-check-cfg=CHECK_CFG`
    RustcCheckCfg(String),

    /// `cargo::rustc-env=VAR=VALUE`
    RustcEnv { var: String, value: String },

    /// `cargo::error=MESSAGE`
    Error(String),

    /// `cargo::warning=MESSAGE`
    Warning(String),

    /// `cargo::metadata=KEY=VALUE`
    Metadata { key: String, value: String },

    /// All other lines that are not cargo directives.
    ///
    /// Build scripts can output arbitrary text to stdout for diagnostic purposes.
    /// Cargo only interprets lines starting with `cargo:` as directives and ignores
    /// everything else. Common examples include:
    /// - Debug/diagnostic output (e.g., "Compiling native library...")
    /// - Empty lines
    /// - Rust debug output (e.g., "OUT_DIR = Some(...)")
    /// - Unknown cargo directives (e.g., "cargo:unknown-directive=value")
    /// - Malformed directives (e.g., "cargo:rustc-env=INVALID")
    ///
    /// These lines are preserved as-is during backup and restoration to maintain
    /// the complete output file.
    Other(String),
}

impl BuildScriptOutputLine {
    const RERUN_IF_CHANGED: &str = "cargo:rerun-if-changed";
    const RERUN_IF_ENV_CHANGED: &str = "cargo:rerun-if-env-changed";
    const RUSTC_LINK_ARG: &str = "cargo:rustc-link-arg";
    const RUSTC_LINK_LIB: &str = "cargo:rustc-link-lib";
    const RUSTC_LINK_SEARCH: &str = "cargo:rustc-link-search";
    const RUSTC_FLAGS: &str = "cargo:rustc-flags";
    const RUSTC_CFG: &str = "cargo:rustc-cfg";
    const RUSTC_CHECK_CFG: &str = "cargo:rustc-check-cfg";
    const RUSTC_ENV: &str = "cargo:rustc-env";
    const ERROR: &str = "cargo:error";
    const WARNING: &str = "cargo:warning";
    const METADATA: &str = "cargo:metadata";

    /// Parse a line of the build script file.
    #[instrument(name = "BuildScriptOutputLine::parse")]
    pub async fn parse(profile: &ProfileDir<'_, Locked>, line: &str) -> Self {
        match Self::parse_inner(profile, line).await {
            Ok(parsed) => parsed,
            Err(err) => {
                trace!(?line, ?err, "failed to parse build script output line");
                Self::Other(line.to_string())
            }
        }
    }

    /// Inner fallible parser for cargo directives.
    async fn parse_inner(profile: &ProfileDir<'_, Locked>, line: &str) -> Result<Self> {
        let Some((key, value)) = line.split_once('=') else {
            return Err(eyre!("line does not contain '='"));
        };

        match key {
            Self::RERUN_IF_CHANGED => {
                let path = QualifiedPath::parse(profile, value).await?;
                Ok(Self::RerunIfChanged(path))
            }
            Self::RERUN_IF_ENV_CHANGED => Ok(Self::RerunIfEnvChanged(String::from(value))),
            Self::RUSTC_LINK_ARG => Ok(Self::RustcLinkArg(String::from(value))),
            Self::RUSTC_LINK_LIB => Ok(Self::RustcLinkLib(String::from(value))),
            Self::RUSTC_LINK_SEARCH => {
                if let Some((kind, path_str)) = value.split_once('=') {
                    let path = QualifiedPath::parse(profile, path_str).await?;
                    Ok(Self::RustcLinkSearch {
                        kind: Some(String::from(kind)),
                        path,
                    })
                } else {
                    let path = QualifiedPath::parse(profile, value).await?;
                    Ok(Self::RustcLinkSearch { kind: None, path })
                }
            }
            Self::RUSTC_FLAGS => Ok(Self::RustcFlags(String::from(value))),
            Self::RUSTC_CFG => Ok(Self::RustcCfg(String::from(value))),
            Self::RUSTC_CHECK_CFG => Ok(Self::RustcCheckCfg(String::from(value))),
            Self::RUSTC_ENV => {
                if let Some((var, env_value)) = value.split_once('=') {
                    Ok(Self::RustcEnv {
                        var: String::from(var),
                        value: String::from(env_value),
                    })
                } else {
                    bail!("rustc-env directive missing second '='")
                }
            }
            Self::ERROR => Ok(Self::Error(String::from(value))),
            Self::WARNING => Ok(Self::Warning(String::from(value))),
            Self::METADATA => {
                if let Some((meta_key, meta_value)) = value.split_once('=') {
                    Ok(Self::Metadata {
                        key: String::from(meta_key),
                        value: String::from(meta_value),
                    })
                } else {
                    bail!("metadata directive missing second '='")
                }
            }
            _ => bail!("unknown cargo directive: {key}"),
        }
    }

    /// Reconstruct the line in the current context.
    #[instrument(name = "BuildScriptOutputLine::reconstruct")]
    pub fn reconstruct(&self, profile: &ProfileDir<'_, Locked>) -> String {
        match self {
            Self::RerunIfChanged(path) => {
                format!("{}={}", Self::RERUN_IF_CHANGED, path.reconstruct(profile))
            }
            Self::RerunIfEnvChanged(var) => format!("{}={}", Self::RERUN_IF_ENV_CHANGED, var),
            Self::RustcLinkArg(flag) => format!("{}={}", Self::RUSTC_LINK_ARG, flag),
            Self::RustcLinkLib(lib) => format!("{}={}", Self::RUSTC_LINK_LIB, lib),
            Self::RustcLinkSearch {
                kind: Some(k),
                path,
            } => format!(
                "{}={}={}",
                Self::RUSTC_LINK_SEARCH,
                k,
                path.reconstruct(profile)
            ),
            Self::RustcLinkSearch { kind: None, path } => {
                format!("{}={}", Self::RUSTC_LINK_SEARCH, path.reconstruct(profile))
            }
            Self::RustcFlags(flags) => format!("{}={}", Self::RUSTC_FLAGS, flags),
            Self::RustcCfg(cfg) => format!("{}={}", Self::RUSTC_CFG, cfg),
            Self::RustcCheckCfg(check_cfg) => format!("{}={}", Self::RUSTC_CHECK_CFG, check_cfg),
            Self::RustcEnv { var, value } => {
                format!("{}={}={}", Self::RUSTC_ENV, var, value)
            }
            Self::Error(msg) => format!("{}={}", Self::ERROR, msg),
            Self::Warning(msg) => format!("{}={}", Self::WARNING, msg),
            Self::Metadata { key, value } => {
                format!("{}={}={}", Self::METADATA, key, value)
            }
            Self::Other(s) => s.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cargo::Profile;
    use pretty_assertions::assert_eq as pretty_assert_eq;

    async fn test_profile() -> ProfileDir<'static, Locked> {
        let workspace = Box::leak(Box::new(
            super::super::workspace::Workspace::from_argv(&[])
                .await
                .unwrap(),
        ));
        workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn parses_rerun_if_changed() {
        let profile = test_profile().await;
        let line = format!("cargo:rerun-if-changed={}/out/build.rs", profile.root());
        let parsed = BuildScriptOutputLine::parse(&profile, &line).await;

        match parsed {
            BuildScriptOutputLine::RerunIfChanged(_) => {}
            _ => panic!("Expected RerunIfChanged variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rerun_if_env_changed() {
        let profile = test_profile().await;
        let line = "cargo:rerun-if-env-changed=RUST_LOG";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RerunIfEnvChanged(var) => {
                pretty_assert_eq!(var, "RUST_LOG");
            }
            _ => panic!("Expected RerunIfEnvChanged variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_link_arg() {
        let profile = test_profile().await;
        let line = "cargo:rustc-link-arg=-Wl,-rpath,/custom/path";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RustcLinkArg(flag) => {
                pretty_assert_eq!(flag, "-Wl,-rpath,/custom/path");
            }
            _ => panic!("Expected RustcLinkArg variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_link_lib() {
        let profile = test_profile().await;
        let line = "cargo:rustc-link-lib=ssl";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RustcLinkLib(lib) => {
                pretty_assert_eq!(lib, "ssl");
            }
            _ => panic!("Expected RustcLinkLib variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_link_search_without_kind() {
        let profile = test_profile().await;
        let line = format!("cargo:rustc-link-search={}/native", profile.root());
        let parsed = BuildScriptOutputLine::parse(&profile, &line).await;

        match &parsed {
            BuildScriptOutputLine::RustcLinkSearch { kind, path: _ } => {
                pretty_assert_eq!(kind, &None);
            }
            _ => panic!("Expected RustcLinkSearch variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_link_search_with_kind() {
        let profile = test_profile().await;
        let line = format!("cargo:rustc-link-search=native={}/lib", profile.root());
        let parsed = BuildScriptOutputLine::parse(&profile, &line).await;

        match &parsed {
            BuildScriptOutputLine::RustcLinkSearch { kind, path: _ } => {
                pretty_assert_eq!(kind, &Some(String::from("native")));
            }
            _ => panic!("Expected RustcLinkSearch variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_flags() {
        let profile = test_profile().await;
        let line = "cargo:rustc-flags=-l dylib=foo";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RustcFlags(flags) => {
                pretty_assert_eq!(flags, "-l dylib=foo");
            }
            _ => panic!("Expected RustcFlags variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_cfg() {
        let profile = test_profile().await;
        let line = "cargo:rustc-cfg=feature=\"custom\"";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RustcCfg(cfg) => {
                pretty_assert_eq!(cfg, "feature=\"custom\"");
            }
            _ => panic!("Expected RustcCfg variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_check_cfg() {
        let profile = test_profile().await;
        let line = "cargo:rustc-check-cfg=cfg(foo)";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RustcCheckCfg(check_cfg) => {
                pretty_assert_eq!(check_cfg, "cfg(foo)");
            }
            _ => panic!("Expected RustcCheckCfg variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_rustc_env() {
        let profile = test_profile().await;
        let line = "cargo:rustc-env=FOO=bar";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::RustcEnv { var, value } => {
                pretty_assert_eq!(var, "FOO");
                pretty_assert_eq!(value, "bar");
            }
            _ => panic!("Expected RustcEnv variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_error() {
        let profile = test_profile().await;
        let line = "cargo:error=Something went wrong";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::Error(msg) => {
                pretty_assert_eq!(msg, "Something went wrong");
            }
            _ => panic!("Expected Error variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_warning() {
        let profile = test_profile().await;
        let line = "cargo:warning=This is a warning";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::Warning(msg) => {
                pretty_assert_eq!(msg, "This is a warning");
            }
            _ => panic!("Expected Warning variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_metadata() {
        let profile = test_profile().await;
        let line = "cargo:metadata=key=value";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match &parsed {
            BuildScriptOutputLine::Metadata { key, value } => {
                pretty_assert_eq!(key, "key");
                pretty_assert_eq!(value, "value");
            }
            _ => panic!("Expected Metadata variant"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, line);
    }

    #[tokio::test]
    async fn parses_other_lines() {
        let profile = test_profile().await;

        let lines = vec![
            "OUT_DIR = Some(/path/to/out)",
            "cargo:unknown=value",
            "random text",
            "",
        ];

        for line in lines {
            let parsed = BuildScriptOutputLine::parse(&profile, line).await;

            match &parsed {
                BuildScriptOutputLine::Other(content) => {
                    pretty_assert_eq!(content, line);
                }
                _ => panic!("Expected Other variant for line: {}", line),
            }

            let reconstructed = parsed.reconstruct(&profile);
            pretty_assert_eq!(reconstructed, line);
        }
    }

    #[tokio::test]
    async fn parses_rustc_env_without_equals() {
        let profile = test_profile().await;
        let line = "cargo:rustc-env=INVALID";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match parsed {
            BuildScriptOutputLine::Other(content) => {
                pretty_assert_eq!(content, line);
            }
            _ => panic!("Expected Other variant for malformed rustc-env"),
        }
    }

    #[tokio::test]
    async fn parses_metadata_without_equals() {
        let profile = test_profile().await;
        let line = "cargo:metadata=INVALID";
        let parsed = BuildScriptOutputLine::parse(&profile, line).await;

        match parsed {
            BuildScriptOutputLine::Other(content) => {
                pretty_assert_eq!(content, line);
            }
            _ => panic!("Expected Other variant for malformed metadata"),
        }
    }

    #[tokio::test]
    async fn parses_and_reconstructs_real_world_example_1() {
        let profile = test_profile().await;

        let fixture = include_str!("fixtures/build_script_output_1.txt");
        let input = fixture.replace("__PROFILE_ROOT__", &profile.root().to_string());

        let parsed = BuildScriptOutput(
            futures::stream::iter(input.lines())
                .then(|line| BuildScriptOutputLine::parse(&profile, line))
                .collect::<Vec<_>>()
                .await,
        );

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, input.trim_end());
    }

    #[tokio::test]
    async fn parses_and_reconstructs_real_world_example_2() {
        let profile = test_profile().await;

        let fixture = include_str!("fixtures/build_script_output_2.txt");
        let input = fixture.replace("__PROFILE_ROOT__", &profile.root().to_string());

        let parsed = BuildScriptOutput(
            futures::stream::iter(input.lines())
                .then(|line| BuildScriptOutputLine::parse(&profile, line))
                .collect::<Vec<_>>()
                .await,
        );

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, input.trim_end());
    }

    #[tokio::test]
    async fn parses_and_reconstructs_mixed_content() {
        let profile = test_profile().await;

        let fixture = include_str!("fixtures/build_script_output_mixed.txt");
        let input = fixture.replace("__PROFILE_ROOT__", &profile.root().to_string());

        let parsed = BuildScriptOutput(
            futures::stream::iter(input.lines())
                .then(|line| BuildScriptOutputLine::parse(&profile, line))
                .collect::<Vec<_>>()
                .await,
        );

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, input.trim_end());
    }
}
