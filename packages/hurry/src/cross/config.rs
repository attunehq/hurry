//! Cross.toml configuration management for RUSTC_BOOTSTRAP passthrough.
//!
//! This module provides utilities for managing Cross.toml configuration to
//! ensure RUSTC_BOOTSTRAP environment variable is passed through to Docker
//! containers, which is required for using unstable features like --build-plan.

use std::path::{Path, PathBuf};

use color_eyre::{Result, eyre::Context};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::debug;

use crate::path::AbsDirPath;

#[cfg(test)]
use tempfile::TempDir;

/// RAII guard that manages Cross.toml configuration for RUSTC_BOOTSTRAP
/// passthrough.
///
/// This guard ensures that the Cross.toml file has the necessary configuration
/// to pass through RUSTC_BOOTSTRAP to Docker containers. It handles three
/// scenarios:
///
/// 1. No Cross.toml exists: Creates a temporary config, removes it on drop
/// 2. Cross.toml exists without RUSTC_BOOTSTRAP: Backs up, modifies, restores
///    on drop
/// 3. Cross.toml exists with RUSTC_BOOTSTRAP: No-op (doesn't touch the file)
#[derive(Debug)]
pub struct CrossConfigGuard {
    /// Path to the Cross.toml file
    config_path: PathBuf,
    /// Backup path if we modified an existing file
    backup_path: Option<PathBuf>,
    /// Whether we created the file (vs. modified existing)
    created: bool,
}

impl CrossConfigGuard {
    /// Set up Cross.toml configuration for RUSTC_BOOTSTRAP passthrough.
    ///
    /// This analyzes the existing Cross.toml (if any) and modifies or creates
    /// it as needed to ensure RUSTC_BOOTSTRAP is passed through to the
    /// container.
    pub async fn setup(workspace_root: &AbsDirPath) -> Result<Self> {
        let config_path = workspace_root.as_std_path().join("Cross.toml");

        // Check if Cross.toml exists
        if !config_path.exists() {
            // Scenario 1: No Cross.toml - create temporary one
            debug!("creating temporary Cross.toml with RUSTC_BOOTSTRAP passthrough");
            Self::create_temporary_config(&config_path).await?;
            return Ok(Self {
                config_path,
                backup_path: None,
                created: true,
            });
        }

        // Read existing config
        let contents = fs::read_to_string(&config_path)
            .await
            .context("failed to read Cross.toml")?;

        // Parse the config
        let config =
            toml::from_str::<CrossConfig>(&contents).context("failed to parse Cross.toml")?;

        // Check if RUSTC_BOOTSTRAP is already in passthrough
        if Self::has_rustc_bootstrap_passthrough(&config) {
            // Scenario 3: Already has RUSTC_BOOTSTRAP - no-op
            debug!("Cross.toml already has RUSTC_BOOTSTRAP passthrough");
            return Ok(Self {
                config_path,
                backup_path: None,
                created: false,
            });
        }

        // Scenario 2: Needs RUSTC_BOOTSTRAP - backup, modify
        debug!("modifying Cross.toml to add RUSTC_BOOTSTRAP passthrough");
        let backup_path = Self::backup_and_modify(&config_path, config).await?;
        Ok(Self {
            config_path,
            backup_path: Some(backup_path),
            created: false,
        })
    }

    /// Check if the config already has RUSTC_BOOTSTRAP in passthrough
    fn has_rustc_bootstrap_passthrough(config: &CrossConfig) -> bool {
        config
            .build
            .as_ref()
            .and_then(|b| b.env.as_ref())
            .and_then(|e| e.passthrough.as_ref())
            .map(|p| p.iter().any(|v| v == "RUSTC_BOOTSTRAP"))
            .unwrap_or(false)
    }

    /// Create a temporary Cross.toml with RUSTC_BOOTSTRAP passthrough
    async fn create_temporary_config(path: &Path) -> Result<()> {
        let config = CrossConfig {
            build: Some(BuildConfig {
                env: Some(EnvConfig {
                    passthrough: Some(vec![String::from("RUSTC_BOOTSTRAP")]),
                }),
            }),
        };

        let contents = toml::to_string_pretty(&config).context("failed to serialize Cross.toml")?;

        fs::write(path, contents)
            .await
            .context("failed to write temporary Cross.toml")?;

        Ok(())
    }

    /// Backup existing config and modify it to add RUSTC_BOOTSTRAP
    async fn backup_and_modify(path: &Path, mut config: CrossConfig) -> Result<PathBuf> {
        // Create backup path
        let backup_path = path.with_extension("toml.hurry-backup");

        // Backup original file
        fs::copy(path, &backup_path)
            .await
            .context("failed to backup Cross.toml")?;

        // Add RUSTC_BOOTSTRAP to passthrough
        let build = config.build.get_or_insert_with(Default::default);
        let env = build.env.get_or_insert_with(Default::default);
        let passthrough = env.passthrough.get_or_insert_with(Vec::new);

        if !passthrough.contains(&String::from("RUSTC_BOOTSTRAP")) {
            passthrough.push(String::from("RUSTC_BOOTSTRAP"));
        }

        // Write modified config
        let contents =
            toml::to_string_pretty(&config).context("failed to serialize modified Cross.toml")?;

        fs::write(path, contents)
            .await
            .context("failed to write modified Cross.toml")?;

        Ok(backup_path)
    }

    /// Clean up: restore or remove the config file
    async fn cleanup(&mut self) -> Result<()> {
        if self.created {
            // We created the file, remove it
            debug!("removing temporary Cross.toml");
            if self.config_path.exists() {
                fs::remove_file(&self.config_path)
                    .await
                    .context("failed to remove temporary Cross.toml")?;
            }
        } else if let Some(backup_path) = &self.backup_path {
            // We modified an existing file, restore from backup
            debug!("restoring original Cross.toml from backup");
            if backup_path.exists() {
                fs::rename(backup_path, &self.config_path)
                    .await
                    .context("failed to restore Cross.toml from backup")?;
            }
        }
        Ok(())
    }
}

impl Drop for CrossConfigGuard {
    fn drop(&mut self) {
        // Best effort cleanup - log errors but don't panic
        if (self.created || self.backup_path.is_some())
            && let Err(e) = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(self.cleanup())
            })
        {
            debug!(?e, "failed to cleanup Cross.toml on drop");
        }
    }
}

/// Minimal Cross.toml configuration structure
#[derive(Debug, Default, Deserialize, Serialize)]
struct CrossConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<BuildConfig>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct BuildConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<EnvConfig>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct EnvConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    passthrough: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_workspace() -> (TempDir, AbsDirPath) {
        let temp = TempDir::new().unwrap();
        let path = AbsDirPath::try_from(temp.path().to_path_buf()).unwrap();
        (temp, path)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn creates_config_when_missing() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        // Guard should create the file
        {
            let _guard = CrossConfigGuard::setup(&workspace).await.unwrap();
            assert!(config_path.exists());

            // Verify it has RUSTC_BOOTSTRAP
            let contents = fs::read_to_string(&config_path).await.unwrap();
            assert!(contents.contains("RUSTC_BOOTSTRAP"));
        }

        // After drop, file should be removed
        assert!(!config_path.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn preserves_existing_config_with_bootstrap() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        // Create config that already has RUSTC_BOOTSTRAP
        let config = CrossConfig {
            build: Some(BuildConfig {
                env: Some(EnvConfig {
                    passthrough: Some(vec![String::from("RUSTC_BOOTSTRAP")]),
                }),
            }),
        };
        let original = toml::to_string_pretty(&config).unwrap();
        fs::write(&config_path, &original).await.unwrap();

        // Guard should not modify it
        {
            let _guard = CrossConfigGuard::setup(&workspace).await.unwrap();
            let contents = fs::read_to_string(&config_path).await.unwrap();
            assert_eq!(contents, original);
        }

        // After drop, file should be unchanged
        let contents = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(contents, original);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adds_bootstrap_to_existing_config() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        // Create config without RUSTC_BOOTSTRAP
        let config = CrossConfig {
            build: Some(BuildConfig {
                env: Some(EnvConfig {
                    passthrough: Some(vec![String::from("OTHER_VAR")]),
                }),
            }),
        };
        let original = toml::to_string_pretty(&config).unwrap();
        fs::write(&config_path, &original).await.unwrap();

        // Guard should modify it
        {
            let _guard = CrossConfigGuard::setup(&workspace).await.unwrap();
            let contents = fs::read_to_string(&config_path).await.unwrap();
            assert!(contents.contains("RUSTC_BOOTSTRAP"));
            assert!(contents.contains("OTHER_VAR"));
        }

        // After drop, original should be restored
        let contents = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(contents, original);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleanup_removes_temporary_config() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        let mut guard = CrossConfigGuard::setup(&workspace).await.unwrap();
        assert!(config_path.exists());

        guard.cleanup().await.unwrap();
        assert!(!config_path.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleanup_restores_modified_config() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        // Create config without RUSTC_BOOTSTRAP
        let original = "[build]\n[build.env]\npassthrough = [\"OTHER_VAR\"]\n";
        fs::write(&config_path, original).await.unwrap();

        let mut guard = CrossConfigGuard::setup(&workspace).await.unwrap();
        let modified = fs::read_to_string(&config_path).await.unwrap();
        assert_ne!(modified, original);

        guard.cleanup().await.unwrap();
        let restored = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(restored, original);
    }
}
