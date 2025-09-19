//! Exercises e2e functionality for building/caching third-party dependencies
//! inside a debian docker container.

use std::path::PathBuf;

use color_eyre::Result;
use e2e::{
    Build, Command, Container,
    ext::{ArtifactIterExt, MessageIterExt},
};
use itertools::Itertools;
use simple_test_case::test_case;

/// Exercises building and caching the project in a single directory.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test(tokio::test)]
async fn same_dir(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;

    let pwd = PathBuf::from("/");
    let container = Container::debian_rust()
        .command(Command::clone_hurry(&pwd))
        .command(Command::install_hurry(pwd.join("hurry")))
        .start()
        .await?;

    // Nothing should be cached on the first build.
    let repo_root = pwd.join(repo);
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;

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
    Command::cargo_clean(&repo_root)
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;
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
#[test_log::test(tokio::test)]
async fn cross_dir(username: &str, repo: &str, branch: &str) -> Result<()> {
    let pwd = PathBuf::from("/");
    let container = Container::debian_rust()
        .command(Command::clone_hurry(&pwd))
        .command(Command::install_hurry(pwd.join("hurry")))
        .start()
        .await?;

    // Nothing should be cached on the first build.
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(pwd.join(repo))
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;
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

    // Now if we clone the repo to a new directory and rebuild, `hurry` should
    // reuse the cache and enable fresh artifacts.
    let repo2 = pwd.join(format!("{repo}-2"));
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .dir(&repo2)
        .finish()
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(&repo2)
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;
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
#[test_case("attunehq", "hurry-tests", "test/native", "tiny"; "attunehq/hurry-tests:test/native")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main", "attune"; "attunehq/attune:main"))]
#[test_log::test(tokio::test)]
async fn native(username: &str, repo: &str, branch: &str, bin: &str) -> Result<()> {
    let pwd = PathBuf::from("/");
    let container = Container::debian_rust()
        .command(
            Command::new()
                .pwd(&pwd)
                .name("apt-get")
                .arg("update")
                .finish(),
        )
        .command(
            Command::new()
                .pwd(&pwd)
                .name("apt-get")
                .arg("install")
                .arg("-y")
                .arg("libgpg-error-dev")
                .arg("libgpgme-dev")
                .arg("pkg-config")
                .finish(),
        )
        .command(Command::clone_hurry(&pwd))
        .command(Command::install_hurry(pwd.join("hurry")))
        .start()
        .await?;

    // Nothing should be cached on the first build.
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(pwd.join(repo))
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;
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

    // We test that we can actually run the binary because the test cases
    // contain dynamically linked native libraries.
    Command::new()
        .pwd(&pwd)
        .name(pwd.join(repo).join("target").join("debug").join(bin))
        .arg("--help")
        .finish()
        .run_docker(&container)
        .await?;

    // Now if we clone the repo to a new directory and rebuild, `hurry` should
    // reuse the cache and enable fresh artifacts.
    let repo2 = format!("{repo}-2");
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .dir(&repo2)
        .finish()
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(pwd.join(&repo2))
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;
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

    // And we should still be able to run the binary.
    Command::new()
        .pwd(&pwd)
        .name(pwd.join(&repo2).join("target").join("debug").join(bin))
        .arg("--help")
        .finish()
        .run_docker(&container)
        .await?;

    Ok(())
}

/// Exercises building and caching the project with native dependencies that are
/// uninstalled between the first and second build. The goal of this test is to
/// prove that the build _fails to compile_ despite the dependency being
/// restored.
#[test_case("attunehq", "hurry-tests", "test/native", "tiny"; "attunehq/hurry-tests:test/native")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main", "attune"; "attunehq/attune:main"))]
#[test_log::test(tokio::test)]
async fn native_uninstalled(username: &str, repo: &str, branch: &str, bin: &str) -> Result<()> {
    let pwd = PathBuf::from("/");
    let container = Container::debian_rust()
        .command(
            Command::new()
                .pwd(&pwd)
                .name("apt-get")
                .arg("update")
                .finish(),
        )
        .command(
            Command::new()
                .pwd(&pwd)
                .name("apt-get")
                .arg("install")
                .arg("-y")
                .arg("libgpg-error-dev")
                .arg("libgpgme-dev")
                .arg("pkg-config")
                .finish(),
        )
        .command(Command::clone_hurry(&pwd))
        .command(Command::install_hurry(pwd.join("hurry")))
        .start()
        .await?;

    // Nothing should be cached on the first build.
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_docker(&container)
        .await?;
    let messages = Build::new()
        .pwd(pwd.join(repo))
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await?;
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

    // We test that we can actually run the binary because the test cases
    // contain dynamically linked native libraries.
    Command::new()
        .pwd(&pwd)
        .name(pwd.join(repo).join("target").join("debug").join(bin))
        .arg("--help")
        .finish()
        .run_docker(&container)
        .await?;

    // We uninstall the native dependencies we installed earlier.
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("remove")
        .arg("-y")
        .arg("libgpg-error-dev")
        .arg("libgpgme-dev")
        .arg("pkg-config")
        .finish()
        .run_docker(&container)
        .await?;

    // Now if we clone the repo to a new directory and rebuild, `hurry` should
    // reuse the cache, which theoretically would enable fresh artifacts...
    let repo2 = format!("{repo}-2");
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .dir(&repo2)
        .finish()
        .run_docker(&container)
        .await?;

    // ... but since we uninstalled the native dependencies, the build should
    // actually fail to compile.
    let build = Build::new()
        .pwd(pwd.join(&repo2))
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container)
        .await;
    assert!(build.is_err(), "build should fail: {build:?}");

    Ok(())
}
