//! Exercises e2e functionality for building/caching third-party dependencies
//! inside a debian docker container.

use std::path::PathBuf;

use color_eyre::Result;
use e2e::{
    Build, Command, Container,
    ext::{ArtifactIterExt, MessageIterExt},
    temporary_directory,
};
use itertools::Itertools;
use location_macros::workspace_dir;
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
        .volume_bind(workspace_dir!(), "/hurry-workspace")
        .command(Command::install_hurry("/hurry-workspace"))
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
        .volume_bind(workspace_dir!(), "/hurry-workspace")
        .command(Command::install_hurry("/hurry-workspace"))
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
        .volume_bind(workspace_dir!(), "/hurry-workspace")
        .command(Command::install_hurry("/hurry-workspace"))
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
        .volume_bind(workspace_dir!(), "/hurry-workspace")
        .command(Command::install_hurry("/hurry-workspace"))
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

/// Exercises building and caching the project across containers with a shared
/// volume.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test(tokio::test)]
async fn cross_container(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;

    // This temporary directory holds the hurry cache, which in this test will
    // be shared across containers.
    let temp_cache = temporary_directory()?;
    let cache_host_path = temp_cache.path().to_string_lossy().to_string();
    let cache_container_path = String::from("/hurry-cache");
    let pwd = PathBuf::from("/");

    // We also make the directories in which the project is cloned different in
    // each container just to be sure that nothing is accidentally getting
    // reused there; in effect this test is a strict superset of `cross_dir`.
    let pwd_repo_a = pwd.join(format!("{repo}-container-a"));
    let pwd_repo_b = pwd.join(format!("{repo}-container-b"));

    // Note: we keep the first container alive until the end instead of putting
    // it in a scope so that the shared volume is preserved. We also create both
    // containers here so that we can do so concurrently and reduce overall test
    // runtime- container creation time is mostly bounded on "how fast does
    // hurry build" but this saves a few seconds at least for effectively no
    // real cost.
    let (container_a, container_b) = tokio::try_join!(
        Container::debian_rust()
            .volume_bind(workspace_dir!(), "/hurry-workspace")
            .command(Command::install_hurry("/hurry-workspace"))
            .command(
                Command::clone_github()
                    .pwd(&pwd)
                    .user(username)
                    .repo(repo)
                    .branch(branch)
                    .dir(&pwd_repo_a)
                    .finish()
            )
            .volume_bind(&cache_host_path, &cache_container_path)
            .start(),
        Container::debian_rust()
            .volume_bind(workspace_dir!(), "/hurry-workspace")
            .command(Command::install_hurry("/hurry-workspace"))
            .command(
                Command::clone_github()
                    .pwd(&pwd)
                    .user(username)
                    .repo(repo)
                    .branch(branch)
                    .dir(&pwd_repo_b)
                    .finish()
            )
            .volume_bind(&cache_host_path, &cache_container_path)
            .start(),
    )?;

    // Nothing should be cached on the first build.
    let messages_a = Build::new()
        .pwd(&pwd_repo_a)
        .env("HOME", &cache_container_path)
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container_a)
        .await?;
    let expected = messages_a
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, false))
        .sorted()
        .collect::<Vec<_>>();
    let freshness = messages_a
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh in container A: {messages_a:?}"
    );

    // Now if we set up a new container with the same cache rebuild, `hurry`
    // should reuse the cache and enable fresh artifacts.
    let messages_b = Build::new()
        .pwd(&pwd_repo_b)
        .env("HOME", &cache_container_path)
        .wrapper(Build::HURRY_NAME)
        .finish()
        .run_docker(&container_b)
        .await?;
    let expected = messages_b
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, true))
        .sorted()
        .collect::<Vec<_>>();
    let freshness = messages_b
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh in container B: {messages_b:?}"
    );

    Ok(())
}

/// Exercises building and caching the project concurrently across containers
/// with shared volume. This test verifies that hurry's cache locking works
/// correctly when multiple containers build simultaneously from the same cache
/// directory.
///
/// Important distinction: this test really validates that the cache being
/// shared and built concurrently doesn't result in any _corruption_ of the
/// cache or any failed builds; the current design of `hurry` only checks and
/// restores from the cache at the very beginning of the build so it does not
/// benefit at all from running builds concurrently.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test(tokio::test)]
async fn cross_container_concurrent(username: &str, repo: &str, branch: &str) -> Result<()> {
    let _ = color_eyre::install()?;

    // This temporary directory holds the hurry cache, which in this test will
    // be shared across containers that build concurrently.
    let temp_cache = temporary_directory()?;
    let cache_host_path = temp_cache.path().to_string_lossy().to_string();
    let cache_container_path = String::from("/hurry-cache");
    let pwd = PathBuf::from("/");

    // We also make the directories in which the project is cloned different in
    // each container just to be sure that nothing is accidentally getting
    // reused there; in effect this test is a strict superset of `cross_dir`.
    let pwd_repo_a = pwd.join(format!("{repo}-concurrent-a"));
    let pwd_repo_b = pwd.join(format!("{repo}-concurrent-b"));
    let (container_a, container_b) = tokio::try_join!(
        Container::debian_rust()
            .volume_bind(workspace_dir!(), "/hurry-workspace")
            .command(Command::install_hurry("/hurry-workspace"))
            .command(
                Command::clone_github()
                    .pwd(&pwd)
                    .user(username)
                    .repo(repo)
                    .branch(branch)
                    .dir(&pwd_repo_a)
                    .finish()
            )
            .volume_bind(&cache_host_path, &cache_container_path)
            .start(),
        Container::debian_rust()
            .volume_bind(workspace_dir!(), "/hurry-workspace")
            .command(Command::install_hurry("/hurry-workspace"))
            .command(
                Command::clone_github()
                    .pwd(&pwd)
                    .user(username)
                    .repo(repo)
                    .branch(branch)
                    .dir(&pwd_repo_b)
                    .finish()
            )
            .volume_bind(&cache_host_path, &cache_container_path)
            .start(),
    )?;

    // This is the main part of the test: both containers should be able to
    // build simultaneously without corrupting the shared cache.
    let build_a = Build::new()
        .pwd(&pwd_repo_a)
        .env("HOME", &cache_container_path)
        .wrapper(Build::HURRY_NAME)
        .finish();
    let build_b = Build::new()
        .pwd(&pwd_repo_b)
        .env("HOME", &cache_container_path)
        .wrapper(Build::HURRY_NAME)
        .finish();
    let (messages_a, messages_b) = tokio::try_join!(
        build_a.run_docker(&container_a),
        build_b.run_docker(&container_b)
    )?;

    let packages_a = messages_a
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .sorted()
        .collect::<Vec<_>>();
    let packages_b = messages_b
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        packages_a,
        packages_b,
        "both concurrent builds should have built the same packages"
    );

    // We don't actually expect any packages to have been fresh for either build
    // since the current design of `hurry` only checks and restores from the
    // cache at the very beginning. However, we don't assert to that effect
    // because if we make this better in the future it shouldn't mean the test
    // now fails.
    //
    // The best way to know if the cache was actually written properly is to try
    // to run the builds again (again concurrently) and see if all the artifacts
    // are fresh. They definitely should be at this point.
    let build_a = Build::new()
        .pwd(&pwd_repo_a)
        .env("HOME", &cache_container_path)
        .wrapper(Build::HURRY_NAME)
        .finish();
    let build_b = Build::new()
        .pwd(&pwd_repo_b)
        .env("HOME", &cache_container_path)
        .wrapper(Build::HURRY_NAME)
        .finish();
    let (messages_a, messages_b) = tokio::try_join!(
        build_a.run_docker(&container_a),
        build_b.run_docker(&container_b)
    )?;
    let packages_a = messages_a
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .sorted()
        .collect::<Vec<_>>();
    let packages_b = messages_b
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        packages_a,
        packages_b,
        "both concurrent builds should have built the same packages"
    );

    let expected_a = messages_a
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, true))
        .sorted()
        .collect::<Vec<_>>();
    let freshness_a = messages_a
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected_a,
        freshness_a,
        "all artifacts should be fresh: {messages_a:?}"
    );

    let expected_b = messages_b
        .iter()
        .thirdparty_artifacts()
        .package_ids()
        .map(|id| (id, true))
        .sorted()
        .collect::<Vec<_>>();
    let freshness_b = messages_b
        .iter()
        .thirdparty_artifacts()
        .freshness()
        .sorted()
        .collect::<Vec<_>>();
    pretty_assertions::assert_eq!(
        expected_b,
        freshness_b,
        "all artifacts should be fresh: {messages_b:?}"
    );

    Ok(())
}
