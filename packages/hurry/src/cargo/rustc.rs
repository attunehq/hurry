use std::{
    collections::BTreeMap,
    fmt::Debug,
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, eyre},
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::path::{AbsDirPath, TryJoinWith as _};

/// Rust compiler metadata for cache key generation.
///
/// Contains platform-specific compiler information needed to generate cache
/// keys that are valid only for the current compilation target. This ensures
/// cached artifacts are not incorrectly shared between different platforms or
/// compiler configurations.
///
/// Currently only captures the LLVM target triple, but could be extended to
/// include compiler version, feature flags, or other compilation options that
/// affect output compatibility.
//
// TODO: Support users cross compiling; probably need to parse argv?
//
// TODO: Determine minimum compiler version.
//
// TODO: Is there a better way to get this?
//
// TODO: Add output from `rustc -vV`, which is what Cargo invokes? How does
// Cargo use this information?
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize)]
pub struct RustcMetadata {
    /// The host target triple.
    #[serde(rename = "llvm-target")]
    pub host_target: String,
}

impl RustcMetadata {
    /// Get platform metadata from the current compiler.
    #[instrument(name = "RustcMetadata::from_argv")]
    pub async fn from_argv(workspace_root: &AbsDirPath, _argv: &[String]) -> Result<Self> {
        // TODO: Is this the correct `rustc` to use? Do we need to specially
        // handle interactions with `rustup` and `rust-toolchain.toml`?
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

/// Records the raw `rustc` invocation information.
#[derive(Debug, Serialize, Deserialize)]
pub struct RawRustcInvocation {
    pub timestamp: SystemTime,
    pub invocation: Vec<String>,
    // Use BTreeMap instead of HashMap so the JSON is sorted.
    pub env: BTreeMap<String, String>,
    pub cwd: String,
}

pub const INVOCATION_LOG_DIR_ENV_VAR: &str = "HURRY_CARGO_INVOCATION_LOG_DIR";

pub fn invocation_log_dir(workspace_target_dir: &AbsDirPath) -> AbsDirPath {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time should be after Unix epoch");
    workspace_target_dir
        .try_join_dirs(["hurry", "rustc", &timestamp.as_nanos().to_string()])
        .expect("rustc invocation log dir should be valid")
}

pub struct RustcInvocation {}

impl RustcInvocation {
    pub fn from_argv(argv: &[String]) -> Self {
        Self {}
    }
}
