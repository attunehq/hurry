//! Workspace extensions for cross-compilation support.
//!
//! This module provides extensions to the `Workspace` type to support
//! cross-compilation via the `cross` tool. The main difference from regular
//! cargo builds is that cross runs inside a Docker container, which means:
//!
//! 1. Build plans must be executed through the cross container
//! 2. Container paths (e.g., `/target/...`) must be converted to host paths
//! 3. Cross.toml configuration is needed for RUSTC_BOOTSTRAP passthrough

use std::fmt::Debug;

use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context as _, eyre},
};
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    cargo::{BuildPlan, CargoBuildArguments, UnitPlan, Workspace},
    cross::{self, CrossConfigGuard},
    fs,
    path::TryJoinWith as _,
};

/// Convert a Docker container path to a host filesystem path.
///
/// During `cross` builds, we need to generate the `cargo` build plan inside the
/// container, but then use that to interact with artifacts on the local system.
/// This function translates the paths reported by the build plan inside the
/// container to paths on the local system.
///
/// Cross mounts the workspace root at `/project` and the target directory at
/// `/target` inside the container. We need to convert these container paths
/// back to host paths.
///
/// # Examples
///
/// ```ignore
/// let ws = Workspace { build_dir: "/Users/jess/project/target", .. };
///
/// // Container target paths get converted to host paths
/// assert_eq!(
///     convert_container_path_to_host("/target/debug/libfoo.rlib", &ws),
///     "/Users/jess/project/target/debug/libfoo.rlib"
/// );
///
/// // Other absolute paths are preserved
/// assert_eq!(
///     convert_container_path_to_host("/usr/lib/libsystem.so", &ws),
///     "/usr/lib/libsystem.so"
/// );
///
/// // Relative paths are preserved
/// assert_eq!(
///     convert_container_path_to_host("src/main.rs", &ws),
///     "src/main.rs"
/// );
/// ```
fn convert_container_path_to_host(path: &str, workspace: &Workspace) -> String {
    if let Some(suffix) = path.strip_prefix("/target") {
        format!("{}{}", workspace.build_dir.as_std_path().display(), suffix)
    } else {
        path.to_string()
    }
}

/// Convert all container paths in a build plan to host paths.
///
/// This modifies the build plan in-place, converting:
/// - All output file paths in each invocation
/// - Link target paths (for symlinks)
/// - The program path for build script executions
/// - OUT_DIR environment variable paths
fn convert_build_plan_paths(build_plan: &mut BuildPlan, workspace: &Workspace) {
    for invocation in &mut build_plan.invocations {
        // Convert output file paths
        for output in &mut invocation.outputs {
            *output = convert_container_path_to_host(output, workspace);
        }

        // Convert link target paths (symlinks)
        // links is HashMap<String, String> where keys are targets
        let links = std::mem::take(&mut invocation.links);
        invocation.links = links
            .into_iter()
            .map(|(target, link)| {
                (convert_container_path_to_host(&target, workspace), link)
            })
            .collect();

        // Convert program path (for build script executions)
        invocation.program = convert_container_path_to_host(&invocation.program, workspace);

        // Convert OUT_DIR in environment variables
        if let Some(out_dir) = invocation.env.get("OUT_DIR") {
            let converted = convert_container_path_to_host(out_dir, workspace);
            invocation.env.insert(String::from("OUT_DIR"), converted);
        }
    }
}

impl Workspace {
    /// Compute the unit plans for a cross build.
    ///
    /// This is similar to `units()` but uses `cross_build_plan()` which:
    /// 1. Runs the build plan inside the cross container
    /// 2. Converts container paths to host paths
    ///
    /// Since the paths are converted to host paths before unit parsing,
    /// the rest of the logic is identical to regular cargo builds.
    /// We delegate to the shared `units_from_build_plan()` helper
    /// to avoid code duplication.
    #[instrument(name = "Workspace::cross_units")]
    pub async fn cross_units(
        &self,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<Vec<UnitPlan>> {
        let build_plan = self.cross_build_plan(&args).await?;
        // The rest of the parsing logic is the same as units()
        // because we've already converted the paths to host paths.
        self.units_from_build_plan(build_plan).await
    }

    /// Get the build plan by running `cross build --build-plan`.
    ///
    /// This is similar to the regular `build_plan()` method but with key
    /// differences:
    ///
    /// 1. The build plan is executed through `cross` (inside a Docker container)
    /// 2. Container paths are converted to host paths after parsing
    /// 3. Cross.toml is configured to pass through RUSTC_BOOTSTRAP
    ///
    /// # Container Path Conversion
    ///
    /// Cross mounts the target directory at `/target` inside the container.
    /// The build plan will report paths like `/target/debug/libfoo.rlib`,
    /// which we need to convert to the actual host paths like
    /// `/Users/jess/project/target/debug/libfoo.rlib`.
    #[instrument(name = "Workspace::cross_build_plan")]
    pub async fn cross_build_plan(
        &self,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<BuildPlan> {
        // Running `cross build --build-plan` resets the state in the `target`
        // directory, just like cargo. We use the same rename workaround.
        let renamed = if fs::exists(&self.build_dir).await {
            debug!("target exists before running build plan, renaming");
            let temp = self
                .root
                .try_join_dir(format!("target.backup.{}", Uuid::new_v4()))?;

            let renamed = fs::rename(&self.build_dir, &temp).await.is_ok();
            debug!(?renamed, ?temp, "renamed temp target");
            if renamed { Some(temp) } else { None }
        } else {
            debug!("target does not exist before running build plan");
            None
        };

        let ret = self.cross_build_plan_inner(args).await;

        if let Some(temp) = renamed {
            debug!("restoring original target");
            fs::remove_dir_all(&self.build_dir).await?;
            fs::rename(&temp, &self.build_dir).await?;
            debug!("restored original target");
        } else {
            // When the build directory didn't exist at the start, we need to
            // clean up the newly created extraneous build directory.
            debug!(build_dir = ?self.build_dir, "build plan done, cleaning up target");
            fs::remove_dir_all(&self.build_dir).await?;
            debug!("build plan done, done cleaning target");
        }

        ret
    }

    #[instrument(name = "Workspace::cross_build_plan_inner")]
    async fn cross_build_plan_inner(
        &self,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<BuildPlan> {
        // Set up Cross.toml to pass through RUSTC_BOOTSTRAP
        // This guard will clean up when dropped
        let _config_guard = CrossConfigGuard::setup(&self.root)
            .await
            .context("set up Cross.toml configuration")?;

        // Build the arguments for cross build --build-plan
        let mut build_args = args.as_ref().to_argv();
        build_args.extend([
            String::from("--build-plan"),
            String::from("-Z"),
            String::from("unstable-options"),
        ]);

        // Run cross build --build-plan with RUSTC_BOOTSTRAP=1
        let output = cross::invoke_output("build", build_args, [("RUSTC_BOOTSTRAP", "1")])
            .await
            .context("run cross command")?;

        // Parse the build plan from NDJSON output
        // (Same logic as cargo: handle --message-format=json)
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(mut plan) = serde_json::from_str::<BuildPlan>(line) {
                // Convert all container paths to host paths
                convert_build_plan_paths(&mut plan, self);
                return Ok(plan);
            }
        }

        // If we didn't find a valid build plan, return an error with context
        Err(eyre!("no valid build plan found in output"))
            .context("parse build plan")
            .with_section(move || stdout.to_string().header("Stdout:"))
            .with_section(move || {
                String::from_utf8_lossy(&output.stderr)
                    .to_string()
                    .header("Stderr:")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::AbsDirPath;

    fn workspace(root: &str) -> Workspace {
        let root = AbsDirPath::try_from(root).unwrap();
        let build_dir = root.try_join_dir("target").unwrap();
        let cargo_home = root.try_join_dir(".cargo").unwrap();

        Workspace {
            root,
            build_dir,
            cargo_home,
            profile: crate::cargo::Profile::Debug,
            target_arch: crate::cargo::RustcTarget::ImplicitHost,
            host_arch: crate::cargo::RustcTargetPlatform::try_from("x86_64-unknown-linux-gnu")
                .unwrap(),
        }
    }

    #[test]
    fn converts_container_target_path() {
        let ws = workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("/target/debug/libfoo.rlib", &ws),
            "/Users/jess/project/target/debug/libfoo.rlib"
        );
    }

    #[test]
    fn converts_container_target_path_with_triple() {
        let ws = workspace("/home/user/myproject");
        assert_eq!(
            convert_container_path_to_host(
                "/target/x86_64-unknown-linux-gnu/debug/deps/libbar-abc123.rmeta",
                &ws
            ),
            "/home/user/myproject/target/x86_64-unknown-linux-gnu/debug/deps/libbar-abc123.rmeta"
        );
    }

    #[test]
    fn preserves_absolute_paths() {
        let ws = workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("/usr/lib/libfoo.so", &ws),
            "/usr/lib/libfoo.so"
        );
    }

    #[test]
    fn preserves_relative_paths() {
        let ws = workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("src/main.rs", &ws),
            "src/main.rs"
        );
    }

    #[test]
    fn preserves_empty_strings() {
        let ws = workspace("/Users/jess/project");
        assert_eq!(convert_container_path_to_host("", &ws), "");
    }

    #[test]
    fn handles_target_in_middle_of_path() {
        // Paths with /target in the middle (not at start) should not be converted
        let ws = workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("/some/target/path", &ws),
            "/some/target/path"
        );
    }
}
