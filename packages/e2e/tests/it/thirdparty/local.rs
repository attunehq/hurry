//! Exercises e2e functionality for building/caching third-party dependencies on
//! the local machine.

use std::path::PathBuf;

use cargo_metadata::Message;
use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use e2e::{
    Build, Command,
    ext::{ArtifactIterExt, MessageIterExt},
    temporary_directory,
};
use escargot::CargoBuild;
use itertools::Itertools;
use location_macros::workspace_dir;
use simple_test_case::test_case;

/// Exercises building and caching the project in a single directory.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test]
fn same_dir(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_home = temporary_directory()?;
    let temp_ws = temporary_directory()?;
    let project_root = temp_ws.path().join(repo);
    Command::clone_github(username, repo, temp_ws.path(), branch).run_local()?;

    // Nothing should be cached on the first build.
    let hurry = Build::hurry(workspace_dir!());
    let messages = Build::new()
        .pwd(&project_root)
        .env("HOME", temp_home.path())
        .finish()
        .hurry_local(&hurry)?;
    let expected = messages
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, false))
        .sorted()
        .collect::<Vec<_>>();
    let freshness = messages
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh: {messages:?}"
    );

    // Now if we delete the `target/` directory and rebuild, `hurry` should
    // reuse the cache and enable fresh artifacts.
    Command::cargo_clean(&project_root).run_local()?;
    let messages = Build::new()
        .pwd(&project_root)
        .env("HOME", temp_home.path())
        .finish()
        .hurry_local(&hurry)?;
    let expected = messages
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, true))
        .sorted()
        .collect::<Vec<_>>();
    let freshness = messages
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh: {messages:?}"
    );

    Ok(())
}

/// Exercises building and caching the project across directories.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test]
fn cross_dir(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;
    let temp_home = temporary_directory()?;
    let hurry = Build::hurry(workspace_dir!());

    // This scope ensures that the first workspace is deleted from disk before
    // we try to build the second workspace.
    {
        let temp_ws_1 = temporary_directory()?;
        let project_root_1 = temp_ws_1.path().join(repo);
        Command::clone_github(username, repo, temp_ws_1.path(), branch).run_local()?;
        let messages = Build::new()
            .pwd(&project_root_1)
            .env("HOME", temp_home.path())
            .finish()
            .hurry_local(&hurry)?;
        let expected = messages
            .iter()
            .thirdparty_artifacts()
            .package_ids()
            .map(|id| (id, false))
            .sorted()
            .collect::<Vec<_>>();
        let freshness = messages
            .iter()
            .thirdparty_artifacts()
            .freshness()
            .sorted()
            .collect::<Vec<_>>();
        pretty_assertions::assert_eq!(
            expected,
            freshness,
            "no artifacts should be fresh: {messages:?}"
        );
    }

    let temp_ws_2 = temporary_directory()?;
    let project_root_2 = temp_ws_2.path().join(repo);
    Command::clone_github(username, repo, temp_ws_2.path(), branch).run_local()?;
    let messages = Build::new()
        .pwd(&project_root_2)
        .env("HOME", temp_home.path())
        .finish()
        .hurry_local(&hurry)?;
    let expected = messages
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, true))
        .sorted()
        .collect::<Vec<_>>();
    let freshness = messages
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh: {messages:?}"
    );

    Ok(())
}

/// Exercises building and caching the project with native dependencies.
#[test_case("attunehq", "hurry-tests", "test/native", "tiny", "tiny"; "attunehq/hurry-tests:test/native")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main", "attune", "attune"; "attunehq/attune:main"))]
#[test_log::test]
fn native(username: &str, repo: &str, branch: &str, package: &str, bin: &str) -> Result<()> {
    let _ = color_eyre::install()?;
    let temp_home = temporary_directory()?;
    let hurry = Build::hurry(workspace_dir!());

    // This scope ensures that the first workspace is deleted from disk before
    // we try to build the second workspace.
    {
        let temp_ws_1 = temporary_directory()?;
        let project_root_1 = temp_ws_1.path().join(repo);
        Command::clone_github(username, repo, temp_ws_1.path(), branch).run_local()?;
        let messages = Build::new()
            .pwd(&project_root_1)
            .env("HOME", temp_home.path())
            .finish()
            .hurry_local(&hurry)?;
        let expected = messages
            .iter()
            .thirdparty_artifacts()
            .package_ids()
            .map(|id| (id, false))
            .sorted()
            .collect::<Vec<_>>();
        let freshness = messages
            .iter()
            .thirdparty_artifacts()
            .freshness()
            .sorted()
            .collect::<Vec<_>>();
        pretty_assertions::assert_eq!(
            expected,
            freshness,
            "no artifacts should be fresh: {messages:?}"
        );

        let path = project_root_1.to_string_lossy();
        let status = CargoBuild::new()
            .manifest_path(project_root_1.join("Cargo.toml"))
            .bin(bin)
            .package(package)
            .run()
            .with_context(|| format!("run cargo build in {path}"))?
            .command()
            .arg("--help")
            .status()
            .with_context(|| format!("run cargo build in {path}"))?;
        if !status.success() {
            bail!("run cargo build in {path} failed: {status}");
        }
    }

    let temp_ws_2 = temporary_directory()?;
    let project_root_2 = temp_ws_2.path().join(repo);
    Command::clone_github(username, repo, temp_ws_2.path(), branch).run_local()?;
    let messages = Build::new()
        .pwd(&project_root_2)
        .env("HOME", temp_home.path())
        .finish()
        .hurry_local(&hurry)?;
    let expected = messages
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, true))
        .sorted()
        .collect::<Vec<_>>();
    let freshness = messages
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh: {messages:?}"
    );
    let path = project_root_2.to_string_lossy();
    let status = CargoBuild::new()
        .manifest_path(project_root_2.join("Cargo.toml"))
        .bin(bin)
        .package(package)
        .run()
        .with_context(|| format!("run cargo build in {path}"))?
        .command()
        .arg("--help")
        .status()
        .with_context(|| format!("run cargo build in {path}"))?;
    if !status.success() {
        bail!("run cargo build in {path} failed: {status}");
    }

    Ok(())
}

/// Exercises building and caching the project with native dependencies that
/// change in an incompatible way between the first and second build. The goal
/// of this test is to prove that the build _fails to compile_ despite the
/// dependency being restored.
///
/// TODO: Once the cache is able to support actually keying off of this, we
/// should add tests for working builds with changed but still compatible native
/// dependencies.
#[test_case("attunehq", "hurry-tests", "test/native", "tiny", "tiny"; "attunehq/hurry-tests:test/native")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main", "attune", "attune"; "attunehq/attune:main"))]
#[test_log::test]
fn native_changed_breaks_build(
    username: &str,
    repo: &str,
    branch: &str,
    package: &str,
    bin: &str,
) -> Result<()> {
    let _ = color_eyre::install()?;
    let temp_home = temporary_directory()?;
    let temp_native = temporary_directory()?;
    let linkflag = format!("-L native={}", temp_native.path().to_string_lossy());
    let hurry = Build::hurry(workspace_dir!());

    // This scope ensures that the first workspace is deleted from disk before
    // we try to build the second workspace.
    //
    // Note that we set the `RUSTFLAGS` environment variable to include the
    // temporary native directory we plan to override later here so that the
    // compiler doesn't invalidate the cache just due to this setting.
    {
        let temp_ws_1 = temporary_directory()?;
        let project_root_1 = temp_ws_1.path().join(repo);
        Command::clone_github(username, repo, temp_ws_1.path(), branch).run_local()?;
        let messages = Build::new()
            .pwd(&project_root_1)
            .env("HOME", temp_home.path())
            .env("RUSTFLAGS", &linkflag)
            .finish()
            .hurry_local(&hurry)?;
        let expected = messages
            .iter()
            .thirdparty_artifacts()
            .package_ids()
            .map(|id| (id, false))
            .sorted()
            .collect::<Vec<_>>();
        let freshness = messages
            .iter()
            .thirdparty_artifacts()
            .freshness()
            .sorted()
            .collect::<Vec<_>>();
        pretty_assertions::assert_eq!(
            expected,
            freshness,
            "no artifacts should be fresh: {messages:?}"
        );

        let status = CargoBuild::new()
            .manifest_path(project_root_1.join("Cargo.toml"))
            .bin(bin)
            .package(package)
            .run()
            .context("build workspace")?
            .command()
            .arg("--help")
            .status()
            .context("build workspace")?;
        if !status.success() {
            bail!("build workspace failed: {status}");
        }

        // Now, we're using `gpgme` as a native dependency for testing. This library
        // does the right thing and automatically finds a valid location for the
        // native `libgpgme` and `libgpg-error` libraries on the system. But we need
        // to make sure the files change between the first and second build, and we
        // don't actually have the ability to change the link args provided by the
        // build script unless we fork and patch the dependency.
        //
        // Happily, we can actually look at the compiler messages to see what paths
        // were linked, and then we do a bit of a nasty trick: we create a new temp
        // directory and then put dummy files with the same names into them. We then
        // put that directory first in the `LD_LIBRARY_PATH` and then build, which
        // will cause the compiler to prefer the dummy files over the real ones,
        // which then fail to link up as they're not actually valid libraries.
        //
        // Note that we do filter out paths that are inside the workspace,
        // because we don't want to try to override libraries that are
        // _generated_ by build scripts (e.g. stored in the `out/` directory).
        let native_lib_dirs = messages
            .into_iter()
            .filter_map(|m| match m {
                Message::BuildScriptExecuted(script) => Some(script.linked_paths),
                _ => None,
            })
            .flatten()
            .filter_map(|p| p.as_str().strip_prefix("native=").map(PathBuf::from))
            .filter(|p| p.strip_prefix(temp_ws_1.path()).is_err())
            .unique();
        for lib_dir in native_lib_dirs {
            let entries = std::fs::read_dir(&lib_dir)
                .with_context(|| format!("read directory entries: {lib_dir:?}"))?;
            for entry in entries {
                let entry = entry.context("read directory entry")?;
                if entry.metadata().context("get metadata")?.is_dir() {
                    continue;
                }

                let path = entry.path();
                let dst = temp_native.path().join(entry.file_name());
                std::fs::write(&dst, b"dummy").context("write dummy file")?;
                eprintln!("override native libary {path:?}: {dst:?}");
            }
        }
    }

    let temp_ws_2 = temporary_directory()?;
    let project_root_2 = temp_ws_2.path().join(repo);
    Command::clone_github(username, repo, temp_ws_2.path(), branch).run_local()?;

    let build = Build::new()
        .pwd(&project_root_2)
        .env("HOME", temp_home.path())
        .env("RUSTFLAGS", &linkflag)
        .finish()
        .hurry_local(&hurry);
    assert!(build.is_err(), "build should fail: {build:?}");

    Ok(())
}
