use std::{fmt::Debug, str::FromStr};

use color_eyre::{
    Report, Result,
    eyre::{Context, OptionExt, bail, eyre},
};
use derive_more::Display;
use enum_assoc::Assoc;
use futures::{StreamExt, stream};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tap::TapFallible;
use tracing::{instrument, trace};

use crate::{
    Locked,
    cargo::{QualifiedPath, workspace::ProfileDir},
    fs,
    path::AbsFilePath,
};

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

/// The syntax style used for cargo build script directives.
///
/// Cargo supports both old single-colon syntax (`cargo:`) and current
/// double-colon syntax (`cargo::`). Build scripts can mix both styles
/// in the same output file.
#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize, Display, Assoc,
)]
#[display("{}", self.as_str())]
#[func(pub const fn as_str(&self) -> &str)]
pub enum BuildScriptOutputLineStyle {
    /// Old syntax: `cargo:directive=value`
    #[assoc(as_str = Self::PREFIX_OLD)]
    Old,

    /// Current syntax: `cargo::directive=value`
    #[assoc(as_str = Self::PREFIX_CURRENT)]
    Current,
}

impl BuildScriptOutputLineStyle {
    const PREFIX_OLD: &str = "cargo:";
    const PREFIX_CURRENT: &str = "cargo::";

    /// Parse a line prefixed with a style.
    /// Returns the style and the rest of the line after the prefix.
    pub fn parse_line(line: &str) -> Result<(Self, &str)> {
        if let Some(rest) = line.strip_prefix(Self::PREFIX_CURRENT) {
            Ok((Self::Current, rest))
        } else if let Some(rest) = line.strip_prefix(Self::PREFIX_OLD) {
            Ok((Self::Old, rest))
        } else {
            bail!(
                "line does not start with a known prefix: [{:?}, {:?}]",
                Self::PREFIX_OLD,
                Self::PREFIX_CURRENT
            );
        }
    }
}

impl FromStr for BuildScriptOutputLineStyle {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            Self::PREFIX_OLD => Ok(Self::Old),
            Self::PREFIX_CURRENT => Ok(Self::Current),
            _ => bail!("invalid build script output line style: {s}"),
        }
    }
}

/// Build scripts communicate with Cargo by printing to stdout; this type
/// describes the possible kinds of lines that can be printed.
///
/// Reference for possible options according to the Cargo docs:
/// https://doc.rust-lang.org/cargo/reference/build-scripts.html#outputs-of-the-build-script
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub enum BuildScriptOutputLine {
    /// `cargo::rerun-if-changed=PATH`
    RerunIfChanged(BuildScriptOutputLineStyle, QualifiedPath),

    /// `cargo::rerun-if-env-changed=VAR`
    RerunIfEnvChanged(BuildScriptOutputLineStyle, String),

    /// `cargo::rustc-link-arg=FLAG`
    RustcLinkArg(BuildScriptOutputLineStyle, String),

    /// `cargo::rustc-link-lib=LIB`
    RustcLinkLib(BuildScriptOutputLineStyle, String),

    /// `cargo::rustc-link-search=[KIND=]PATH`
    RustcLinkSearch {
        style: BuildScriptOutputLineStyle,
        kind: Option<String>,
        path: QualifiedPath,
    },

    /// `cargo::rustc-flags=FLAGS`
    RustcFlags(BuildScriptOutputLineStyle, String),

    /// `cargo::rustc-cfg=KEY[="VALUE"]`
    RustcCfg {
        style: BuildScriptOutputLineStyle,
        key: String,
        value: Option<String>,
    },

    /// `cargo::rustc-check-cfg=CHECK_CFG`
    RustcCheckCfg(BuildScriptOutputLineStyle, String),

    /// `cargo::rustc-env=VAR=VALUE`
    RustcEnv {
        style: BuildScriptOutputLineStyle,
        var: String,
        value: String,
    },

    /// `cargo::error=MESSAGE`
    Error(BuildScriptOutputLineStyle, String),

    /// `cargo::warning=MESSAGE`
    Warning(BuildScriptOutputLineStyle, String),

    /// `cargo::metadata=KEY=VALUE`
    Metadata {
        style: BuildScriptOutputLineStyle,
        key: String,
        value: String,
    },

    /// All other lines that are not cargo directives.
    ///
    /// Build scripts can output arbitrary text to stdout for diagnostic
    /// purposes. Cargo only interprets lines starting with `cargo:` as
    /// directives and ignores everything else. Common examples include:
    /// - Debug/diagnostic output (e.g., "Compiling native library...")
    /// - Empty lines
    /// - Rust debug output (e.g., "OUT_DIR = Some(...)")
    /// - Unknown cargo directives (e.g., "cargo:unknown-directive=value")
    /// - Malformed directives (e.g., "cargo:rustc-env=INVALID")
    ///
    /// These lines are preserved as-is during backup and restoration to
    /// maintain the complete output file.
    Other(String),
}

impl BuildScriptOutputLine {
    const RERUN_IF_CHANGED: &str = "rerun-if-changed";
    const RERUN_IF_ENV_CHANGED: &str = "rerun-if-env-changed";
    const RUSTC_LINK_ARG: &str = "rustc-link-arg";
    const RUSTC_LINK_LIB: &str = "rustc-link-lib";
    const RUSTC_LINK_SEARCH: &str = "rustc-link-search";
    const RUSTC_FLAGS: &str = "rustc-flags";
    const RUSTC_CFG: &str = "rustc-cfg";
    const RUSTC_CHECK_CFG: &str = "rustc-check-cfg";
    const RUSTC_ENV: &str = "rustc-env";
    const ERROR: &str = "error";
    const WARNING: &str = "warning";
    const METADATA: &str = "metadata";

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
        let (style, line) = BuildScriptOutputLineStyle::parse_line(line)?;
        let Some((key, value)) = line.split_once('=') else {
            return Err(eyre!("directive does not contain '='"));
        };

        match key {
            Self::RERUN_IF_CHANGED => {
                let path = QualifiedPath::parse(profile, value).await?;
                Ok(Self::RerunIfChanged(style, path))
            }
            Self::RERUN_IF_ENV_CHANGED => Ok(Self::RerunIfEnvChanged(style, String::from(value))),
            Self::RUSTC_LINK_ARG => Ok(Self::RustcLinkArg(style, String::from(value))),
            Self::RUSTC_LINK_LIB => Ok(Self::RustcLinkLib(style, String::from(value))),
            Self::RUSTC_LINK_SEARCH => {
                if let Some((kind, path)) = value.split_once('=') {
                    Ok(Self::RustcLinkSearch {
                        style,
                        kind: Some(String::from(kind)),
                        path: QualifiedPath::parse(profile, path).await?,
                    })
                } else {
                    Ok(Self::RustcLinkSearch {
                        style,
                        kind: None,
                        path: QualifiedPath::parse(profile, value).await?,
                    })
                }
            }
            Self::RUSTC_FLAGS => Ok(Self::RustcFlags(style, String::from(value))),
            Self::RUSTC_CFG => {
                if let Some((key, value)) = value.split_once('=') {
                    Ok(Self::RustcCfg {
                        style,
                        key: String::from(key),
                        value: Some(String::from(value)),
                    })
                } else {
                    Ok(Self::RustcCfg {
                        style,
                        key: String::from(value),
                        value: None,
                    })
                }
            }
            Self::RUSTC_CHECK_CFG => Ok(Self::RustcCheckCfg(style, String::from(value))),
            Self::RUSTC_ENV => {
                if let Some((var, value)) = value.split_once('=') {
                    Ok(Self::RustcEnv {
                        style,
                        var: String::from(var),
                        value: String::from(value),
                    })
                } else {
                    bail!("rustc-env directive missing second '='")
                }
            }
            Self::ERROR => Ok(Self::Error(style, String::from(value))),
            Self::WARNING => Ok(Self::Warning(style, String::from(value))),
            Self::METADATA => {
                if let Some((key, value)) = value.split_once('=') {
                    Ok(Self::Metadata {
                        style,
                        key: String::from(key),
                        value: String::from(value),
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
            Self::RerunIfChanged(style, path) => {
                format!(
                    "{}{}={}",
                    style.as_str(),
                    Self::RERUN_IF_CHANGED,
                    path.reconstruct(profile)
                )
            }
            Self::RerunIfEnvChanged(style, var) => {
                format!("{}{}={}", style.as_str(), Self::RERUN_IF_ENV_CHANGED, var)
            }
            Self::RustcLinkArg(style, flag) => {
                format!("{}{}={}", style.as_str(), Self::RUSTC_LINK_ARG, flag)
            }
            Self::RustcLinkLib(style, lib) => {
                format!("{}{}={}", style.as_str(), Self::RUSTC_LINK_LIB, lib)
            }
            Self::RustcLinkSearch { style, kind, path } => match kind {
                Some(kind) => format!(
                    "{}{}={}={}",
                    style.as_str(),
                    Self::RUSTC_LINK_SEARCH,
                    kind,
                    path.reconstruct(profile)
                ),
                None => format!(
                    "{}{}={}",
                    style.as_str(),
                    Self::RUSTC_LINK_SEARCH,
                    path.reconstruct(profile)
                ),
            },
            Self::RustcFlags(style, flags) => {
                format!("{}{}={}", style.as_str(), Self::RUSTC_FLAGS, flags)
            }
            Self::RustcCfg { style, key, value } => match value {
                None => {
                    format!("{}{}={}", style.as_str(), Self::RUSTC_CFG, key)
                }
                Some(value) => {
                    format!("{}{}={}={}", style.as_str(), Self::RUSTC_CFG, key, value)
                }
            },
            Self::RustcCheckCfg(style, check_cfg) => {
                format!("{}{}={}", style.as_str(), Self::RUSTC_CHECK_CFG, check_cfg)
            }
            Self::RustcEnv { style, var, value } => {
                format!("{}{}={}={}", style.as_str(), Self::RUSTC_ENV, var, value)
            }
            Self::Error(style, msg) => format!("{}{}={}", style.as_str(), Self::ERROR, msg),
            Self::Warning(style, msg) => format!("{}{}={}", style.as_str(), Self::WARNING, msg),
            Self::Metadata { style, key, value } => {
                format!("{}{}={}={}", style.as_str(), Self::METADATA, key, value)
            }
            Self::Other(s) => s.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cargo::{Profile, Workspace};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use simple_test_case::test_case;

    fn replace_path_placeholders(line: &str, profile: &ProfileDir<Locked>) -> String {
        line.replace("__PROFILE__", &profile.root().to_string())
            .replace("__CARGO__", &profile.workspace.cargo_home.to_string())
    }

    #[test_case("cargo:rerun-if-changed=__PROFILE__/out/build.rs", BuildScriptOutputLineStyle::Old, "__PROFILE__/out/build.rs"; "old_style_profile_root")]
    #[test_case("cargo::rerun-if-changed=__PROFILE__/out/build.rs", BuildScriptOutputLineStyle::Current, "__PROFILE__/out/build.rs"; "current_style_profile_root")]
    #[test_case("cargo:rerun-if-changed=__CARGO__/out/build.rs", BuildScriptOutputLineStyle::Old, "__CARGO__/out/build.rs"; "old_style_cargo_home")]
    #[test_case("cargo::rerun-if-changed=__CARGO__/out/build.rs", BuildScriptOutputLineStyle::Current, "__CARGO__/out/build.rs"; "current_style_cargo_home")]
    #[tokio::test]
    async fn parses_rerun_if_changed(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_path: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");
        let line = replace_path_placeholders(line, &profile);
        let expected_path = replace_path_placeholders(expected_path, &profile);

        match BuildScriptOutputLine::parse(&profile, &line).await {
            BuildScriptOutputLine::RerunIfChanged(style, path) => {
                pretty_assert_eq!(path.reconstruct(&profile), expected_path);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RerunIfChanged variant"),
        }
    }

    #[test_case("cargo:rerun-if-env-changed=RUST_LOG", BuildScriptOutputLineStyle::Old, "RUST_LOG"; "old_style")]
    #[test_case("cargo::rerun-if-env-changed=RUST_LOG", BuildScriptOutputLineStyle::Current, "RUST_LOG"; "current_style")]
    #[tokio::test]
    async fn parses_rerun_if_env_changed(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_var: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RerunIfEnvChanged(style, var) => {
                pretty_assert_eq!(var, expected_var);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RerunIfEnvChanged variant"),
        }
    }

    #[test_case("cargo:rustc-link-arg=-Wl,-rpath,/custom/path", BuildScriptOutputLineStyle::Old, "-Wl,-rpath,/custom/path"; "old_style")]
    #[test_case("cargo::rustc-link-arg=-Wl,-rpath,/custom/path", BuildScriptOutputLineStyle::Current, "-Wl,-rpath,/custom/path"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_link_arg(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_flag: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RustcLinkArg(style, flag) => {
                pretty_assert_eq!(flag, expected_flag);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcLinkArg variant"),
        }
    }

    #[test_case("cargo:rustc-link-lib=ssl", BuildScriptOutputLineStyle::Old, "ssl"; "old_style")]
    #[test_case("cargo::rustc-link-lib=ssl", BuildScriptOutputLineStyle::Current, "ssl"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_link_lib(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_lib: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RustcLinkLib(style, lib) => {
                pretty_assert_eq!(lib, expected_lib);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcLinkLib variant"),
        }
    }

    #[test_case("cargo:rustc-link-search=__PROFILE__/native", BuildScriptOutputLineStyle::Old, "__PROFILE__/native"; "old_style")]
    #[test_case("cargo::rustc-link-search=__PROFILE__/native", BuildScriptOutputLineStyle::Current, "__PROFILE__/native"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_link_search_without_kind(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_path: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");
        let line = replace_path_placeholders(line, &profile);
        let expected_path = replace_path_placeholders(expected_path, &profile);

        match BuildScriptOutputLine::parse(&profile, &line).await {
            BuildScriptOutputLine::RustcLinkSearch { style, kind, path } => {
                pretty_assert_eq!(kind, None);
                pretty_assert_eq!(path.reconstruct(&profile), expected_path);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcLinkSearch variant"),
        }
    }

    #[test_case("cargo:rustc-link-search=native=__PROFILE__/lib", BuildScriptOutputLineStyle::Old, "native", "__PROFILE__/lib"; "old_style")]
    #[test_case("cargo::rustc-link-search=native=__PROFILE__/lib", BuildScriptOutputLineStyle::Current, "native", "__PROFILE__/lib"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_link_search_with_kind(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_kind: &str,
        expected_path: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");
        let line = replace_path_placeholders(line, &profile);
        let expected_path = replace_path_placeholders(expected_path, &profile);

        match BuildScriptOutputLine::parse(&profile, &line).await {
            BuildScriptOutputLine::RustcLinkSearch { style, kind, path } => {
                pretty_assert_eq!(kind, Some(String::from(expected_kind)));
                pretty_assert_eq!(path.reconstruct(&profile), expected_path);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcLinkSearch variant"),
        }
    }

    #[test_case("cargo:rustc-flags=-l dylib=foo", BuildScriptOutputLineStyle::Old, "-l dylib=foo"; "old_style")]
    #[test_case("cargo::rustc-flags=-l dylib=foo", BuildScriptOutputLineStyle::Current, "-l dylib=foo"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_flags(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_flags: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RustcFlags(style, flags) => {
                pretty_assert_eq!(flags, expected_flags);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcFlags variant"),
        }
    }

    #[test_case("cargo:rustc-cfg=feature=\"custom\"", BuildScriptOutputLineStyle::Old, "feature", Some("\"custom\""); "old_style_with_value")]
    #[test_case("cargo::rustc-cfg=feature=\"custom\"", BuildScriptOutputLineStyle::Current, "feature", Some("\"custom\""); "current_style_with_value")]
    #[test_case("cargo:rustc-cfg=has_feature", BuildScriptOutputLineStyle::Old, "has_feature", None; "old_style_without_value")]
    #[test_case("cargo::rustc-cfg=has_feature", BuildScriptOutputLineStyle::Current, "has_feature", None; "current_style_without_value")]
    #[tokio::test]
    async fn parses_rustc_cfg(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_key: &str,
        expected_value: Option<&str>,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RustcCfg { style, key, value } => {
                pretty_assert_eq!(key, expected_key);
                pretty_assert_eq!(value.as_deref(), expected_value);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcCfg variant"),
        }
    }

    #[test_case("cargo:rustc-check-cfg=cfg(foo)", BuildScriptOutputLineStyle::Old, "cfg(foo)"; "old_style")]
    #[test_case("cargo::rustc-check-cfg=cfg(foo)", BuildScriptOutputLineStyle::Current, "cfg(foo)"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_check_cfg(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_check_cfg: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RustcCheckCfg(style, check_cfg) => {
                pretty_assert_eq!(check_cfg, expected_check_cfg);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcCheckCfg variant"),
        }
    }

    #[test_case("cargo:rustc-env=FOO=bar", BuildScriptOutputLineStyle::Old, "FOO", "bar"; "old_style")]
    #[test_case("cargo::rustc-env=FOO=bar", BuildScriptOutputLineStyle::Current, "FOO", "bar"; "current_style")]
    #[tokio::test]
    async fn parses_rustc_env(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_var: &str,
        expected_value: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::RustcEnv { style, var, value } => {
                pretty_assert_eq!(var, expected_var);
                pretty_assert_eq!(value, expected_value);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected RustcEnv variant"),
        }
    }

    #[test_case("cargo:error=Something went wrong", BuildScriptOutputLineStyle::Old, "Something went wrong"; "old_style")]
    #[test_case("cargo::error=Something went wrong", BuildScriptOutputLineStyle::Current, "Something went wrong"; "current_style")]
    #[tokio::test]
    async fn parses_error(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_msg: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::Error(style, msg) => {
                pretty_assert_eq!(msg, expected_msg);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test_case("cargo:warning=This is a warning", BuildScriptOutputLineStyle::Old, "This is a warning"; "old_style")]
    #[test_case("cargo::warning=This is a warning", BuildScriptOutputLineStyle::Current, "This is a warning"; "current_style")]
    #[tokio::test]
    async fn parses_warning(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_msg: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::Warning(style, msg) => {
                pretty_assert_eq!(msg, expected_msg);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected Warning variant"),
        }
    }

    #[test_case("cargo:metadata=key=value", BuildScriptOutputLineStyle::Old, "key", "value"; "old_style")]
    #[test_case("cargo::metadata=key=value", BuildScriptOutputLineStyle::Current, "key", "value"; "current_style")]
    #[tokio::test]
    async fn parses_metadata(
        line: &str,
        expected_style: BuildScriptOutputLineStyle,
        expected_key: &str,
        expected_value: &str,
    ) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::Metadata { style, key, value } => {
                pretty_assert_eq!(key, expected_key);
                pretty_assert_eq!(value, expected_value);
                pretty_assert_eq!(style, expected_style);
            }
            _ => panic!("Expected Metadata variant"),
        }
    }

    #[test_case("OUT_DIR = Some(/path/to/out)"; "debug_output")]
    #[test_case("cargo:unknown=value"; "unknown_directive")]
    #[test_case("random text"; "random_text")]
    #[test_case(""; "empty_line")]
    #[tokio::test]
    async fn parses_other_lines(line: &str) {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        match BuildScriptOutputLine::parse(&profile, line).await {
            BuildScriptOutputLine::Other(content) => {
                pretty_assert_eq!(content, line);
            }
            _ => panic!("Expected Other variant for line: {}", line),
        }
    }

    #[tokio::test]
    async fn parses_rustc_env_without_equals() {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");
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
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");
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
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        let fixture = include_str!("fixtures/build_script_output_1.txt");
        let input = replace_path_placeholders(fixture, &profile);

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
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        let fixture = include_str!("fixtures/build_script_output_2.txt");
        let input = replace_path_placeholders(fixture, &profile);

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
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        let fixture = include_str!("fixtures/build_script_output_mixed.txt");
        let input = replace_path_placeholders(fixture, &profile);

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
    async fn parses_and_reconstructs_mixed_styles() {
        let workspace = Workspace::from_argv(CargoBuildArguments::empty())
            .await
            .expect("open current workspace");
        let profile = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .expect("open profile");

        let fixture = include_str!("fixtures/build_script_output_mixed_styles.txt");
        let input = replace_path_placeholders(fixture, &profile);

        let parsed = BuildScriptOutput(
            futures::stream::iter(input.lines())
                .then(|line| BuildScriptOutputLine::parse(&profile, line))
                .collect::<Vec<_>>()
                .await,
        );

        // Verify we parsed both old and current styles
        let lines = &parsed.0;
        match &lines[0] {
            BuildScriptOutputLine::RustcCfg { style, .. } => {
                pretty_assert_eq!(style, &BuildScriptOutputLineStyle::Old);
            }
            _ => panic!("Expected RustcCfg with Old style"),
        }
        match &lines[1] {
            BuildScriptOutputLine::RustcCfg { style, .. } => {
                pretty_assert_eq!(style, &BuildScriptOutputLineStyle::Current);
            }
            _ => panic!("Expected RustcCfg with Current style"),
        }

        let reconstructed = parsed.reconstruct(&profile);
        pretty_assert_eq!(reconstructed, input.trim_end());
    }
}
