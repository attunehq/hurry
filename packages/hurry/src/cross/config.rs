//! Cross.toml configuration management for RUSTC_BOOTSTRAP passthrough.
//!
//! This module provides utilities for managing Cross.toml configuration to
//! ensure RUSTC_BOOTSTRAP environment variable is passed through to Docker
//! containers, which is required for using unstable features like --build-plan.

use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt},
};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::fs;
use tracing::debug;

use crate::path::AbsDirPath;

/// Configuration for cross invocation with RUSTC_BOOTSTRAP passthrough.
///
/// This struct manages a temporary Cross.toml configuration file that ensures
/// RUSTC_BOOTSTRAP is passed through to Docker containers.
///
/// The temp file is automatically cleaned up when this struct is dropped.
#[derive(Debug)]
pub struct CrossConfig {
    /// The temp file containing the modified config.
    /// None if the original Cross.toml already has RUSTC_BOOTSTRAP configured.
    temp_file: NamedTempFile,

    /// The stringified path to the temp file.
    ///
    /// We need to be able to set the path as an env variable to `cross`, which
    /// means it needs to be a string. But in order to do that, we need to parse
    /// the path as UTF8. We do that at creation time because there's no world
    /// in which a `CrossConfig` is created without reading its path as a string
    /// to pass as an environment variable to `cross`, so this way we can report
    /// any errors parsing as UTF8 at the same time as any other errors.
    path: String,
}

impl CrossConfig {
    /// Set up Cross.toml configuration for RUSTC_BOOTSTRAP passthrough.
    ///
    /// This analyzes the existing Cross.toml (if any) and creates a temporary
    /// config file with RUSTC_BOOTSTRAP passthrough if needed.
    ///
    /// Returns a CrossConfig that should be kept alive for the duration of the
    /// cross invocation. Use `cross_config_path()` to get the path to pass
    /// via the CROSS_CONFIG environment variable.
    ///
    /// TODO: There are other ways that users could specify `Cross.toml`
    /// configurations, we should handle those to correctly merge configuration
    /// keys.
    pub async fn setup(workspace_root: &AbsDirPath) -> Result<Option<Self>> {
        let config_path = workspace_root.as_std_path().join("Cross.toml");

        if !config_path.exists() {
            debug!("creating temporary Cross.toml with RUSTC_BOOTSTRAP passthrough");
            let (temp_file, path) = Self::create_temp_config(CrossToml::default()).await?;
            return Ok(Some(Self { temp_file, path }));
        }

        let contents = fs::read_to_string(&config_path)
            .await
            .context("read Cross.toml")?;

        let config = toml::from_str::<CrossToml>(&contents).context("parse Cross.toml")?;
        if Self::has_rustc_bootstrap_passthrough(&config) {
            debug!("Cross.toml already has RUSTC_BOOTSTRAP passthrough");
            return Ok(None);
        }

        debug!("creating temporary Cross.toml with RUSTC_BOOTSTRAP added");
        let (temp_file, path) = Self::create_temp_config(config).await?;
        Ok(Some(Self { temp_file, path }))
    }

    /// Get the path to use for CROSS_CONFIG environment variable.
    pub fn path(&self) -> &str {
        &self.path
    }

    fn has_rustc_bootstrap_passthrough(config: &CrossToml) -> bool {
        config
            .build
            .as_ref()
            .and_then(|b| b.env.as_ref())
            .and_then(|e| e.passthrough.as_ref())
            .is_some_and(|p| p.iter().any(|v| v == "RUSTC_BOOTSTRAP"))
    }

    async fn create_temp_config(mut config: CrossToml) -> Result<(NamedTempFile, String)> {
        let build = config.build.get_or_insert_with(Default::default);
        let env = build.env.get_or_insert_with(Default::default);
        let passthrough = env.passthrough.get_or_insert_with(Vec::new);

        if !passthrough.contains(&String::from("RUSTC_BOOTSTRAP")) {
            passthrough.push(String::from("RUSTC_BOOTSTRAP"));
        }

        let contents = toml::to_string_pretty(&config).context("serialize Cross.toml")?;
        let temp_file = NamedTempFile::with_suffix(".toml").context("create temp file")?;
        let path = temp_file
            .path()
            .to_str()
            .ok_or_eyre("path is not valid UTF8")
            .with_section(|| format!("{temp_file:?}").header("Temporary file path:"))
            .map(String::from)?;

        fs::write(temp_file.path(), contents)
            .await
            .context("write temp Cross.toml")?;

        Ok((temp_file, path))
    }
}

/// Minimal Cross.toml configuration structure
#[derive(Debug, Default, Deserialize, Serialize)]
struct CrossToml {
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
    use tempfile::TempDir;

    use super::*;

    async fn temp_workspace() -> (TempDir, AbsDirPath) {
        let temp = TempDir::new().unwrap();
        let path = AbsDirPath::try_from(temp.path().to_path_buf()).unwrap();
        (temp, path)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn creates_temp_config_when_missing() {
        let (_temp, workspace) = temp_workspace().await;
        let original_config_path = workspace.as_std_path().join("Cross.toml");

        let config = CrossConfig::setup(&workspace)
            .await
            .expect("set up cross config")
            .expect("create cross config temp file");

        // Should have a temp file
        let temp_path = config.temp_file.path();
        assert!(
            temp_path.exists(),
            "Temp file should exist at {temp_path:?}"
        );

        // The path should match
        assert_eq!(
            temp_path.to_string_lossy().as_ref(),
            config.path(),
            "Temp file path should be the same as returned by the `path` method"
        );

        // Temp file should have RUSTC_BOOTSTRAP
        let contents = fs::read_to_string(&temp_path).await.unwrap();
        assert!(contents.contains("RUSTC_BOOTSTRAP"));

        // Original location should NOT have a file
        assert!(!original_config_path.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_temp_file_when_already_configured() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        // Create existing config with RUSTC_BOOTSTRAP
        let existing = CrossToml {
            build: Some(BuildConfig {
                env: Some(EnvConfig {
                    passthrough: Some(vec![String::from("RUSTC_BOOTSTRAP")]),
                }),
            }),
        };
        let contents = toml::to_string_pretty(&existing).unwrap();
        fs::write(&config_path, &contents).await.unwrap();

        let config = CrossConfig::setup(&workspace)
            .await
            .expect("set up cross config");

        // Should NOT have a temp file - original is fine
        assert!(
            config.is_none(),
            "CrossConfig should be None when Cross.toml already has RUSTC_BOOTSTRAP"
        );

        // Original should be unchanged
        let after = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(after, contents);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn creates_temp_with_merged_config() {
        let (_temp, workspace) = temp_workspace().await;
        let config_path = workspace.as_std_path().join("Cross.toml");

        // Create existing config WITHOUT RUSTC_BOOTSTRAP
        let existing = CrossToml {
            build: Some(BuildConfig {
                env: Some(EnvConfig {
                    passthrough: Some(vec![String::from("OTHER_VAR")]),
                }),
            }),
        };
        let original = toml::to_string_pretty(&existing).unwrap();
        fs::write(&config_path, &original).await.unwrap();

        let config = CrossConfig::setup(&workspace)
            .await
            .expect("set up cross config")
            .expect("create cross config temp file");

        // Should have a temp file
        let temp_path = config.temp_file.path();
        assert!(
            temp_path.exists(),
            "Temp file should exist at {temp_path:?}"
        );

        // The path should match
        assert_eq!(
            temp_path.to_string_lossy().as_ref(),
            config.path(),
            "Temp file path should be the same as returned by the `path` method"
        );

        // Temp should have both vars
        let temp_contents = fs::read_to_string(&temp_path).await.unwrap();
        assert!(
            temp_contents.contains("RUSTC_BOOTSTRAP"),
            "Temp file should contain RUSTC_BOOTSTRAP"
        );
        assert!(
            temp_contents.contains("OTHER_VAR"),
            "Temp file should preserve OTHER_VAR from original config"
        );

        // Original should be unchanged
        let after = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(after, original, "Original Cross.toml should be unchanged");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn temp_file_cleaned_up_on_drop() {
        let (_temp, workspace) = temp_workspace().await;

        let temp_path = {
            let config = CrossConfig::setup(&workspace)
                .await
                .expect("set up cross config")
                .expect("create cross config temp file");

            let path = config.temp_file.path().to_path_buf();
            assert!(path.exists(), "Temp file should exist at {path:?}");
            path
        };

        // After drop, temp file should be gone
        assert!(
            !temp_path.exists(),
            "Temp file should be cleaned up after drop"
        );
    }
}
