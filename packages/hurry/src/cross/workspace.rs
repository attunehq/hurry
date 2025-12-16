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
use tracing::{debug, instrument, trace};
use uuid::Uuid;

use crate::{
    cargo::{BuildPlan, CargoBuildArguments, RustcTargetPlatform, UnitPlan, Workspace},
    cross::{self, CrossConfig},
    fs,
    path::TryJoinWith as _,
};

/// Prefix used by cross to mount the target directory.
const CONTAINER_TARGET_PREFIX: &str = "/target";

/// Prefix used by cross to mount the cargo home directory.
const CONTAINER_CARGO_PREFIX: &str = "/cargo";

/// Prefix used by cross to mount the workspace root.
const CONTAINER_PROJECT_PREFIX: &str = "/project";

/// Convert a Docker container path to a host filesystem path.
///
/// During `cross` builds, we need to generate the `cargo` build plan inside the
/// container, but then use that to interact with artifacts on the local system.
/// This function translates the paths reported by the build plan inside the
/// container to paths on the local system.
///
/// # Container Path Conventions
///
/// Cross v0.2.5 (the current tagged release) mounts directories at fixed paths:
/// - `/project` → workspace root
/// - `/target` → target directory
/// - `/cargo` → cargo home directory
///
/// The untagged development version of cross uses the actual host paths for
/// `/cargo` and `/project`, but still uses `/target` for the target directory.
/// We need to handle both, because 0.2.5 was tagged years ago and developers
/// are increasingly likely to be using the development version due to the work
/// done on it since: https://github.com/cross-rs/cross/issues/1659
#[instrument]
fn convert_container_path_to_host(path: &str, workspace: &Workspace) -> String {
    trace!(?path, "converting container path to host");
    if let Some(suffix) = path.strip_prefix(CONTAINER_TARGET_PREFIX) {
        trace!(?suffix, prefix = ?CONTAINER_TARGET_PREFIX, "stripped target prefix");
        format!("{}{}", workspace.build_dir.as_std_path().display(), suffix)
    } else if let Some(suffix) = path.strip_prefix(CONTAINER_CARGO_PREFIX) {
        trace!(?suffix, prefix = ?CONTAINER_CARGO_PREFIX, "stripped cargo prefix");
        format!("{}{}", workspace.cargo_home.as_std_path().display(), suffix)
    } else if let Some(suffix) = path.strip_prefix(CONTAINER_PROJECT_PREFIX) {
        trace!(?suffix, prefix = ?CONTAINER_PROJECT_PREFIX, "stripped project prefix");
        format!("{}{}", workspace.root.as_std_path().display(), suffix)
    } else {
        trace!(?path, "not a container path, returning unchanged");
        path.to_string()
    }
}

/// Extract the host architecture from the build plan.
///
/// Cross containers have a specific host architecture (typically
/// `x86_64-unknown-linux-gnu`) that may differ from the target. Build script
/// execution units have a `HOST` environment variable that tells us the
/// container's host triple.
///
/// Returns `None` if no build script execution units are found (which would
/// be unusual but possible for projects with no build scripts).
pub fn extract_host_arch(build_plan: &BuildPlan) -> Option<RustcTargetPlatform> {
    for invocation in &build_plan.invocations {
        if invocation.compile_mode == crate::cargo::CargoCompileMode::RunCustomBuild
            && let Some(host) = invocation.env.get("HOST")
            && let Ok(platform) = RustcTargetPlatform::try_from(host.as_str())
        {
            return Some(platform);
        }
    }
    None
}

/// Convert all container paths in a build plan to host paths.
fn convert_build_plan_paths(build_plan: &mut BuildPlan, workspace: &Workspace) {
    for invocation in &mut build_plan.invocations {
        // Convert output paths
        for output in &mut invocation.outputs {
            *output = convert_container_path_to_host(output, workspace);
        }

        // Convert links (HashMap<String, String> where keys are link targets)
        let links = std::mem::take(&mut invocation.links);
        invocation.links = links
            .into_iter()
            .map(|(target, link)| {
                (
                    convert_container_path_to_host(&target, workspace),
                    convert_container_path_to_host(&link, workspace),
                )
            })
            .collect();

        // Convert program path
        invocation.program = convert_container_path_to_host(&invocation.program, workspace);

        // Convert cwd
        invocation.cwd = convert_container_path_to_host(&invocation.cwd, workspace);

        // Convert environment variables that contain paths
        let env_keys_to_convert = ["OUT_DIR", "CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH"];
        for key in env_keys_to_convert {
            if let Some(value) = invocation.env.get(key) {
                let converted = convert_container_path_to_host(value, workspace);
                invocation.env.insert(String::from(key), converted);
            }
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
        self.units_from_build_plan(build_plan).await
    }

    /// Get the build plan by running `cross build --build-plan`.
    ///
    /// This is similar to the regular `build_plan()` method but with key
    /// differences:
    ///
    /// 1. The build plan is executed through `cross` (inside a Docker
    ///    container)
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
        // Set up temporary Cross.toml with RUSTC_BOOTSTRAP passthrough.
        // The config is kept alive for the duration of the cross invocation.
        let cross_config = CrossConfig::setup(&self.root)
            .await
            .context("set up Cross.toml configuration")?;

        let mut build_args = args.as_ref().to_argv();
        build_args.extend([
            String::from("--build-plan"),
            String::from("-Z"),
            String::from("unstable-options"),
        ]);

        // Build env vars: always need RUSTC_BOOTSTRAP, optionally CROSS_CONFIG
        let mut env = vec![(String::from("RUSTC_BOOTSTRAP"), String::from("1"))];
        if let Some(config) = cross_config {
            env.push((String::from("CROSS_CONFIG"), config.path().to_string()));
        }

        let output = cross::invoke_output("build", build_args, env)
            .await
            .context("invoke cross")?;

        // Handle --message-format=json which produces NDJSON output
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(mut plan) = serde_json::from_str::<BuildPlan>(line) {
                convert_build_plan_paths(&mut plan, self);
                return Ok(plan);
            }
        }

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
    use std::collections::HashMap;

    use super::*;
    use crate::{
        cargo::{BuildPlanInvocation, CargoCompileMode},
        path::AbsDirPath,
    };

    fn workspace(root: &str, cargo_home: &str) -> Workspace {
        let root = AbsDirPath::try_from(root).unwrap();
        let build_dir = root.try_join_dir("target").unwrap();
        let cargo_home = AbsDirPath::try_from(cargo_home).unwrap();

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

    // Helper for creating test workspaces with default cargo_home
    fn default_workspace(root: &str) -> Workspace {
        let cargo_home = if root.starts_with("/home/") {
            // Linux-style: /home/user/.cargo
            let user = root.split('/').nth(2).unwrap_or("user");
            format!("/home/{user}/.cargo")
        } else {
            // macOS-style: /Users/user/.cargo
            let user = root.split('/').nth(2).unwrap_or("jess");
            format!("/Users/{user}/.cargo")
        };
        workspace(root, &cargo_home)
    }

    #[test]
    fn converts_container_target_path() {
        let ws = default_workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("/target/debug/libfoo.rlib", &ws),
            "/Users/jess/project/target/debug/libfoo.rlib"
        );
    }

    #[test]
    fn converts_container_target_path_with_triple() {
        let ws = default_workspace("/home/user/myproject");
        assert_eq!(
            convert_container_path_to_host(
                "/target/x86_64-unknown-linux-gnu/debug/deps/libbar-abc123.rmeta",
                &ws
            ),
            "/home/user/myproject/target/x86_64-unknown-linux-gnu/debug/deps/libbar-abc123.rmeta"
        );
    }

    #[test]
    fn converts_container_cargo_path() {
        let ws = workspace("/home/eliza/src/myproject", "/home/eliza/.cargo");
        assert_eq!(
            convert_container_path_to_host(
                "/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.0.0/src/lib.rs",
                &ws
            ),
            "/home/eliza/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.0.0/src/lib.rs"
        );
    }

    #[test]
    fn converts_container_project_path() {
        let ws = default_workspace("/home/user/myproject");
        assert_eq!(
            convert_container_path_to_host("/project/src/main.rs", &ws),
            "/home/user/myproject/src/main.rs"
        );
    }

    #[test]
    fn converts_container_project_subdir_path() {
        let ws = default_workspace("/home/user/myproject");
        assert_eq!(
            convert_container_path_to_host("/project/packages/foo/src/lib.rs", &ws),
            "/home/user/myproject/packages/foo/src/lib.rs"
        );
    }

    #[test]
    fn preserves_absolute_paths() {
        let ws = default_workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("/usr/lib/libfoo.so", &ws),
            "/usr/lib/libfoo.so"
        );
    }

    #[test]
    fn preserves_relative_paths() {
        let ws = default_workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("src/main.rs", &ws),
            "src/main.rs"
        );
    }

    #[test]
    fn preserves_empty_strings() {
        let ws = default_workspace("/Users/jess/project");
        assert_eq!(convert_container_path_to_host("", &ws), "");
    }

    #[test]
    fn handles_target_in_middle_of_path() {
        // Paths with /target in the middle (not at start) should not be converted
        let ws = default_workspace("/Users/jess/project");
        assert_eq!(
            convert_container_path_to_host("/some/target/path", &ws),
            "/some/target/path"
        );
    }

    fn make_invocation(
        cwd: &str,
        outputs: Vec<&str>,
        manifest_dir: Option<&str>,
    ) -> BuildPlanInvocation {
        let mut env = HashMap::new();
        if let Some(dir) = manifest_dir {
            env.insert(String::from("CARGO_MANIFEST_DIR"), String::from(dir));
        }
        BuildPlanInvocation {
            package_name: String::from("test-pkg"),
            package_version: String::from("1.0.0"),
            target_kind: vec![],
            target_arch: crate::cargo::RustcTarget::ImplicitHost,
            compile_mode: CargoCompileMode::Build,
            deps: vec![],
            outputs: outputs.into_iter().map(String::from).collect(),
            links: HashMap::new(),
            program: String::from("rustc"),
            args: vec![],
            env,
            cwd: String::from(cwd),
        }
    }

    #[test]
    fn converts_host_paths_unchanged() {
        // Dev version of cross uses actual host paths - these should pass through
        // unchanged
        let ws = workspace("/home/user/myproject", "/home/user/.cargo");

        // Host cargo path should be unchanged
        assert_eq!(
            convert_container_path_to_host(
                "/home/user/.cargo/registry/src/index.crates.io-xxx/foo-1.0/src/lib.rs",
                &ws
            ),
            "/home/user/.cargo/registry/src/index.crates.io-xxx/foo-1.0/src/lib.rs"
        );

        // Host project path should be unchanged
        assert_eq!(
            convert_container_path_to_host("/home/user/myproject/src/main.rs", &ws),
            "/home/user/myproject/src/main.rs"
        );
    }

    #[test]
    fn converts_mixed_paths_correctly() {
        // Dev version still uses /target, so we need to convert that
        let ws = workspace("/home/user/myproject", "/home/user/.cargo");

        // /target paths still need conversion (used by both versions)
        assert_eq!(
            convert_container_path_to_host("/target/debug/libfoo.rlib", &ws),
            "/home/user/myproject/target/debug/libfoo.rlib"
        );

        // Host paths pass through
        assert_eq!(
            convert_container_path_to_host("/home/user/.cargo/registry/src/foo/lib.rs", &ws),
            "/home/user/.cargo/registry/src/foo/lib.rs"
        );
    }

    fn make_build_script_invocation(host: &str) -> BuildPlanInvocation {
        let mut env = HashMap::new();
        env.insert(String::from("HOST"), String::from(host));
        env.insert(
            String::from("TARGET"),
            String::from("aarch64-unknown-linux-gnu"),
        );
        BuildPlanInvocation {
            package_name: String::from("test-pkg"),
            package_version: String::from("1.0.0"),
            target_kind: vec![],
            target_arch: crate::cargo::RustcTarget::ImplicitHost,
            compile_mode: CargoCompileMode::RunCustomBuild,
            deps: vec![],
            outputs: vec![],
            links: HashMap::new(),
            program: String::from("/target/debug/build/test-pkg/build-script-build"),
            args: vec![],
            env,
            cwd: String::from("/cargo/registry/src/test-pkg"),
        }
    }

    #[test]
    fn extract_host_arch_finds_host_from_build_script() {
        let plan = BuildPlan {
            invocations: vec![make_build_script_invocation("x86_64-unknown-linux-gnu")],
            inputs: vec![],
        };
        let host = extract_host_arch(&plan);
        assert!(host.is_some());
        assert_eq!(host.unwrap().as_str(), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn extract_host_arch_returns_none_without_build_scripts() {
        let plan = BuildPlan {
            invocations: vec![make_invocation(
                "/cargo/registry/src/foo",
                vec!["/target/debug/libfoo.rlib"],
                None,
            )],
            inputs: vec![],
        };
        assert!(extract_host_arch(&plan).is_none());
    }
}
