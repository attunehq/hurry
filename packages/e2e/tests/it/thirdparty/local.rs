//! Exercises e2e functionality for building/caching third-party dependencies on
//! the local machine.

use color_eyre::Result;
use e2e::{
    cargo_clean, clone_github,
    ext::{ArtifactIterExt, MessageIterExt},
    hurry_cargo_build, temporary_directory,
};
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
