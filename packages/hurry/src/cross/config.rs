//! Management of Cross.toml configuration for RUSTC_BOOTSTRAP passthrough.
//!
//! Cross requires a configuration file to pass environment variables through to
//! the Docker container. This module provides utilities to temporarily manage
//! the Cross.toml file to ensure RUSTC_BOOTSTRAP can be passed through for
//! unstable cargo features like `--build-plan`.

use std::path::{Path, PathBuf};

use color_eyre::{Result, eyre::Context};
use tracing::{debug, trace};

const CROSS_CONFIG_NAME: &str = "Cross.toml";
const CROSS_BACKUP_NAME: &str = "Cross.toml.hurry-backup";

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
    /// Cross.toml existed but needed RUSTC_BOOTSTRAP added (backed up to this path).
    Modified { backup_path: PathBuf },
}

/// Guard that ensures Cross.toml is restored to its original state.
pub struct CrossConfigGuard {
    workspace_root: PathBuf,
    state: ConfigState,
}

impl CrossConfigGuard {
    /// Set up Cross.toml for RUSTC_BOOTSTRAP passthrough.
    ///
    /// This function ensures that Cross.toml exists and has the necessary
    /// configuration to pass RUSTC_BOOTSTRAP through to the container.
    ///
    /// Returns a guard that will restore the original state when dropped.
    pub async fn setup(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let config_path = workspace_root.join(CROSS_CONFIG_NAME);

        let state = if config_path.exists() {
            // Cross.toml exists: check if it already has RUSTC_BOOTSTRAP
            let content = tokio::fs::read_to_string(&config_path)
                .await
                .context("reading Cross.toml")?;

            if has_rustc_bootstrap_passthrough(&content) {
                debug!("Cross.toml already has RUSTC_BOOTSTRAP passthrough configured");
                ConfigState::AlreadyConfigured
            } else {
                debug!("Cross.toml exists but needs RUSTC_BOOTSTRAP passthrough");
                // Back up the current config
                let backup_path = workspace_root.join(CROSS_BACKUP_NAME);
                tokio::fs::copy(&config_path, &backup_path)
                    .await
                    .context("backing up Cross.toml")?;
                debug!(?backup_path, "backed up Cross.toml");

                // Add RUSTC_BOOTSTRAP to the config
                let new_content = add_rustc_bootstrap_passthrough(&content)?;
                tokio::fs::write(&config_path, new_content)
                    .await
                    .context("writing updated Cross.toml")?;
                debug!("added RUSTC_BOOTSTRAP passthrough to Cross.toml");

                ConfigState::Modified { backup_path }
            }
        } else {
            debug!("No Cross.toml found, creating temporary one");
            // No Cross.toml: create one with RUSTC_BOOTSTRAP
            tokio::fs::write(&config_path, CROSS_CONFIG_WITH_RUSTC_BOOTSTRAP)
                .await
                .context("creating Cross.toml")?;
            debug!("created temporary Cross.toml");

            ConfigState::Missing
        };

        Ok(Self {
            workspace_root,
            state,
        })
    }

    /// Restore the original Cross.toml state.
    pub async fn restore(&mut self) -> Result<()> {
        let config_path = self.workspace_root.join(CROSS_CONFIG_NAME);

        match &self.state {
            ConfigState::Missing => {
                // Remove the temporary config we created
                if config_path.exists() {
                    tokio::fs::remove_file(&config_path)
                        .await
                        .context("removing temporary Cross.toml")?;
                    debug!("removed temporary Cross.toml");
                }
            }
            ConfigState::AlreadyConfigured => {
                // Nothing to do: config was already correct
                trace!("Cross.toml was already configured, nothing to restore");
            }
            ConfigState::Modified { backup_path } => {
                // Restore from backup
                tokio::fs::rename(backup_path, &config_path)
                    .await
                    .context("restoring Cross.toml from backup")?;
                debug!("restored original Cross.toml from backup");
            }
        }

        Ok(())
    }
}

impl Drop for CrossConfigGuard {
    fn drop(&mut self) {
        // Try to restore synchronously in drop
        let config_path = self.workspace_root.join(CROSS_CONFIG_NAME);

        match &self.state {
            ConfigState::Missing => {
                if config_path.exists() {
                    #[allow(clippy::disallowed_methods, reason = "cannot use async in drop")]
                    let _ = std::fs::remove_file(&config_path);
                }
            }
            ConfigState::AlreadyConfigured => {}
            ConfigState::Modified { backup_path } => {
                #[allow(clippy::disallowed_methods, reason = "cannot use async in drop")]
                let _ = std::fs::rename(backup_path, &config_path);
            }
        }
    }
}

/// Check if a Cross.toml content already has RUSTC_BOOTSTRAP in passthrough.
fn has_rustc_bootstrap_passthrough(content: &str) -> bool {
    // Parse as TOML and check for build.env.passthrough containing "RUSTC_BOOTSTRAP"
    match content.parse::<toml::Table>() {
        Ok(table) => {
            if let Some(build) = table.get("build").and_then(|v| v.as_table()) {
                if let Some(env) = build.get("env").and_then(|v| v.as_table()) {
                    if let Some(passthrough) = env.get("passthrough").and_then(|v| v.as_array()) {
                        return passthrough
                            .iter()
                            .any(|v| v.as_str() == Some("RUSTC_BOOTSTRAP"));
                    }
                }
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
