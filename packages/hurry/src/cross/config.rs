//! Management of Cross.toml configuration for RUSTC_BOOTSTRAP passthrough.
//!
//! Cross requires a configuration file to pass environment variables through to
//! the Docker container. This module provides utilities to temporarily manage
//! the Cross.toml file to ensure RUSTC_BOOTSTRAP can be passed through for
//! unstable cargo features like `--build-plan`.

use color_eyre::{Result, eyre::Context};

use crate::{
    fs::{self, RenameGuard},
    mk_rel_file,
    path::{AbsDirPath, AbsFilePath, JoinWith as _},
};

/// TOML configuration for Cross.toml with RUSTC_BOOTSTRAP passthrough.
const CROSS_CONFIG_WITH_RUSTC_BOOTSTRAP: &str = r#"[build.env]
passthrough = [
    "RUSTC_BOOTSTRAP",
]
"#;

/// Represents the state of Cross.toml before our modifications.
#[derive(Debug)]
enum ConfigState {
    /// No Cross.toml existed.
    Missing,

    /// Cross.toml existed and already had RUSTC_BOOTSTRAP configured.
    AlreadyConfigured,

    /// Cross.toml existed but needed RUSTC_BOOTSTRAP added.
    Modified { backup: RenameGuard },
}

/// Guard that ensures Cross.toml is restored to its original state.
pub struct CrossConfigGuard {
    restored: bool,
    config: AbsFilePath,
    state: ConfigState,
}

impl CrossConfigGuard {
    /// Set up Cross.toml for RUSTC_BOOTSTRAP passthrough.
    ///
    /// This function ensures that Cross.toml exists and has the necessary
    /// configuration to pass RUSTC_BOOTSTRAP through to the container.
    ///
    /// Returns a guard that will restore the original state when dropped.
    pub async fn setup(workspace_root: &AbsDirPath) -> Result<Self> {
        let config = workspace_root.join(mk_rel_file!("Cross.toml"));

        let state = if fs::exists(&config).await {
            let content = fs::must_read_buffered_utf8(&config)
                .await
                .context("reading Cross.toml")?;

            if has_rustc_bootstrap_passthrough(&content) {
                ConfigState::AlreadyConfigured
            } else {
                let backup = fs::rename_temporary(&config)
                    .await
                    .context("backup Cross.toml")?;

                let content = add_rustc_bootstrap_passthrough(&content)?;
                fs::write(&config, content)
                    .await
                    .context("writing updated Cross.toml")?;

                ConfigState::Modified { backup }
            }
        } else {
            fs::write(&config, CROSS_CONFIG_WITH_RUSTC_BOOTSTRAP)
                .await
                .context("creating Cross.toml")?;
            ConfigState::Missing
        };

        Ok(Self {
            restored: false,
            config,
            state,
        })
    }

    /// Restore the original Cross.toml state.
    ///
    /// This is also performed on drop, so you only need to do this if you want
    /// to ensure the synchronous drop IO doesn't get performed or if you want
    /// to handle errors explicitly.
    pub async fn restore(&mut self) -> Result<()> {
        match &mut self.state {
            ConfigState::AlreadyConfigured => {}
            ConfigState::Missing => {
                let _ = tokio::fs::remove_file(&self.config).await;
            }
            ConfigState::Modified { backup } => {
                backup.restore().await?;
            }
        }

        self.restored = true;
        Ok(())
    }
}

#[allow(clippy::disallowed_methods, reason = "cannot use async in drop")]
impl Drop for CrossConfigGuard {
    fn drop(&mut self) {
        if !self.restored {
            // `RenameGuard` will move the file back when it is dropped, so no need to
            // handle the `Modified` case explicitly.
            match &self.state {
                ConfigState::AlreadyConfigured => {}
                ConfigState::Modified { .. } => {}
                ConfigState::Missing => {
                    let _ = std::fs::remove_file(&self.config);
                }
            }
        }
    }
}

/// Check if a Cross.toml content already has RUSTC_BOOTSTRAP in passthrough.
fn has_rustc_bootstrap_passthrough(content: &str) -> bool {
    // Parse as TOML and check for build.env.passthrough containing
    // "RUSTC_BOOTSTRAP"
    match content.parse::<toml::Table>() {
        Ok(table) => {
            if let Some(build) = table.get("build").and_then(|v| v.as_table())
                && let Some(env) = build.get("env").and_then(|v| v.as_table())
                && let Some(passthrough) = env.get("passthrough").and_then(|v| v.as_array())
            {
                return passthrough
                    .iter()
                    .any(|v| v.as_str() == Some("RUSTC_BOOTSTRAP"));
            }
            false
        }
        Err(_) => {
            // If we can't parse it, assume it doesn't have the config
            false
        }
    }
}

/// Add RUSTC_BOOTSTRAP to the passthrough list in a Cross.toml content.
fn add_rustc_bootstrap_passthrough(content: &str) -> Result<String> {
    let mut table: toml::Table = content.parse().context("parsing existing Cross.toml")?;

    // Get or create build.env.passthrough
    let build = table
        .entry("build")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| color_eyre::eyre::eyre!("build is not a table"))?;

    let env = build
        .entry("env")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| color_eyre::eyre::eyre!("build.env is not a table"))?;

    let passthrough = env
        .entry("passthrough")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| color_eyre::eyre::eyre!("build.env.passthrough is not an array"))?;

    // Add RUSTC_BOOTSTRAP if not already present
    if !passthrough
        .iter()
        .any(|v| v.as_str() == Some("RUSTC_BOOTSTRAP"))
    {
        passthrough.push(toml::Value::String("RUSTC_BOOTSTRAP".to_string()));
    }

    // Serialize back to TOML
    toml::to_string_pretty(&table).context("serializing updated Cross.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_rustc_bootstrap_passthrough() {
        let with_config = r#"
[build.env]
passthrough = [
    "RUSTC_BOOTSTRAP",
]
"#;
        assert!(has_rustc_bootstrap_passthrough(with_config));

        let with_other = r#"
[build.env]
passthrough = [
    "OTHER_VAR",
]
"#;
        assert!(!has_rustc_bootstrap_passthrough(with_other));

        let empty = "";
        assert!(!has_rustc_bootstrap_passthrough(empty));
    }

    #[test]
    fn test_add_rustc_bootstrap_passthrough() {
        let empty = "";
        let result = add_rustc_bootstrap_passthrough(empty).unwrap();
        assert!(has_rustc_bootstrap_passthrough(&result));

        let with_other = r#"
[build.env]
passthrough = [
    "OTHER_VAR",
]
"#;
        let result = add_rustc_bootstrap_passthrough(with_other).unwrap();
        assert!(has_rustc_bootstrap_passthrough(&result));
        assert!(result.contains("OTHER_VAR"));
    }
}
