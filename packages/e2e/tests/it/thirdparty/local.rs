//! Exercises e2e functionality for building/caching third-party dependencies on
//! the local machine.

use std::path::PathBuf;

use cargo_metadata::Message;
use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use e2e::{
    cargo_clean, clone_github,
    ext::{ArtifactIterExt, MessageIterExt},
    hurry_cargo_build, temporary_directory,
};
use escargot::CargoBuild;
use itertools::Itertools;
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
    clone_github(username, repo, temp_ws.path(), branch)?;

    // Nothing should be cached on the first build.
    let messages = hurry_cargo_build()
        .pwd(temp_ws.path())
        .home(temp_home.path())
        .run()?;
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
    cargo_clean(temp_ws.path())?;
    let messages = hurry_cargo_build()
        .pwd(temp_ws.path())
        .home(temp_home.path())
        .run()?;
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

    // This scope ensures that the first workspace is deleted from disk before
    // we try to build the second workspace.
    {
        let temp_ws_1 = temporary_directory()?;
        clone_github(username, repo, temp_ws_1.path(), branch)?;
        let messages = hurry_cargo_build()
            .pwd(temp_ws_1.path())
            .home(temp_home.path())
            .run()?;
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
    clone_github(username, repo, temp_ws_2.path(), branch)?;
    let messages = hurry_cargo_build()
        .pwd(temp_ws_2.path())
        .home(temp_home.path())
        .run()?;
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
#[test_case("attunehq", "hurry-tests", "test/native"; "attunehq/hurry-tests:test/native")]
#[test_log::test]
fn native(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;
    let temp_home = temporary_directory()?;

    // This scope ensures that the first workspace is deleted from disk before
    // we try to build the second workspace.
    {
        let temp_ws_1 = temporary_directory()?;
        clone_github(username, repo, temp_ws_1.path(), branch)?;
        let messages = hurry_cargo_build()
            .pwd(temp_ws_1.path())
            .home(temp_home.path())
            .run()?;
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

        let path = temp_ws_1.path().to_string_lossy();
        let status = CargoBuild::new()
            .manifest_path(temp_ws_1.path().join("Cargo.toml"))
            .run()
            .with_context(|| format!("run cargo build in {path}"))?
            .command()
            .status()
            .with_context(|| format!("run cargo build in {path}"))?;
        if !status.success() {
            bail!("run cargo build in {path} failed: {status}");
        }
    }

    let temp_ws_2 = temporary_directory()?;
    clone_github(username, repo, temp_ws_2.path(), branch)?;
    let messages = hurry_cargo_build()
        .pwd(temp_ws_2.path())
        .home(temp_home.path())
        .run()?;
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
    let path = temp_ws_2.path().to_string_lossy();
    let status = CargoBuild::new()
        .manifest_path(temp_ws_2.path().join("Cargo.toml"))
        .run()
        .with_context(|| format!("run cargo build in {path}"))?
        .command()
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
#[test_case("attunehq", "hurry-tests", "test/native"; "attunehq/hurry-tests:test/native")]
#[test_log::test]
fn native_changed_breaks_build(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;
    let temp_home = temporary_directory()?;
    let temp_native = temporary_directory()?;
    let linkflag = format!("-L native={}", temp_native.path().to_string_lossy());

    // This scope ensures that the first workspace is deleted from disk before
    // we try to build the second workspace.
    //
    // Note that we set the `RUSTFLAGS` environment variable to include the
    // temporary native directory we plan to override later here so that the
    // compiler doesn't invalidate the cache just due to this setting.
    let build = {
        let temp_ws_1 = temporary_directory()?;
        clone_github(username, repo, temp_ws_1.path(), branch)?;
        let messages = hurry_cargo_build()
            .pwd(temp_ws_1.path())
            .home(temp_home.path())
            .envs(&[("RUSTFLAGS", &linkflag)])
            .run()?;
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
            .manifest_path(temp_ws_1.path().join("Cargo.toml"))
            .run()
            .context("build workspace")?
            .command()
            .status()
            .context("build workspace")?;
        if !status.success() {
            bail!("build workspace failed: {status}");
        }

        messages
    };

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
    let native_lib_dirs = build
        .into_iter()
        .filter_map(|m| match m {
            Message::BuildScriptExecuted(script) => Some(script.linked_paths),
            _ => None,
        })
        .flatten()
        .filter_map(|p| p.as_str().strip_prefix("native=").map(PathBuf::from))
        .unique();
    for lib_dir in native_lib_dirs {
        for entry in std::fs::read_dir(&lib_dir).context("read directory")? {
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

    let temp_ws_2 = temporary_directory()?;
    clone_github(username, repo, temp_ws_2.path(), branch)?;

    let build = hurry_cargo_build()
        .pwd(temp_ws_2.path())
        .home(temp_home.path())
        .envs(&[("RUSTFLAGS", &linkflag)])
        .run();
    assert!(build.is_err(), "build should fail: {build:?}");

    Ok(())
}
