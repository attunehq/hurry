//! Exercises e2e functionality for building/caching third-party dependencies
//! inside a debian docker container.

use std::path::PathBuf;

use color_eyre::{Result, eyre::bail};
use e2e::{
    Build, Command, TestEnv,
    ext::{ArtifactIterExt, MessageIterExt},
};
use itertools::Itertools;
use pretty_assertions::assert_eq as pretty_assert_eq;
use simple_test_case::test_case;

/// Exercises building and caching the project in a single directory.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test(tokio::test)]
async fn same_dir(username: &str, repo: &str, branch: &str) -> Result<()> {
    color_eyre::install()?;

    // Check for GITHUB_TOKEN early to fail fast with a clear error message
    if std::env::var("GITHUB_TOKEN").is_err() {
        bail!(
            "GITHUB_TOKEN environment variable is required to clone repositories from GitHub. \
             Please set it to a personal access token with 'repo' scope."
        );
    }

    // Start test environment with courier
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/workspace");

    // Nothing should be cached on the first build.
    let repo_root = pwd.join(repo);
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
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
    pretty_assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    // Now if we delete the `target/` directory and rebuild, `hurry` should
    // reuse the cache and enable fresh artifacts.
    Command::cargo_clean(&repo_root)
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
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
    pretty_assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    Ok(())
}

/// Exercises building and caching the project across directories.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test(tokio::test)]
async fn cross_dir(username: &str, repo: &str, branch: &str) -> Result<()> {
    color_eyre::install()?;

    // Check for GITHUB_TOKEN early to fail fast with a clear error message
    if std::env::var("GITHUB_TOKEN").is_err() {
        bail!(
            "GITHUB_TOKEN environment variable is required to clone repositories from GitHub. \
             Please set it to a personal access token with 'repo' scope."
        );
    }

    // Start test environment with courier
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/workspace");

    // Nothing should be cached on the first build.
    let repo_root = pwd.join(repo);
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
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
    pretty_assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
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
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo2)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
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
    pretty_assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    Ok(())
}

/// Exercises building and caching the project with native dependencies.
#[test_case("attunehq", "hurry-tests", "test/native", "tiny"; "attunehq/hurry-tests:test/native")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main", "attune"; "attunehq/attune:main"))]
#[test_log::test(tokio::test)]
async fn native(username: &str, repo: &str, branch: &str, bin: &str) -> Result<()> {
    color_eyre::install()?;

    // Check for GITHUB_TOKEN early to fail fast with a clear error message
    if std::env::var("GITHUB_TOKEN").is_err() {
        bail!(
            "GITHUB_TOKEN environment variable is required to clone repositories from GitHub. \
             Please set it to a personal access token with 'repo' scope."
        );
    }

    // Start test environment with courier
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/workspace");

    // Install native dependencies required for building with native libs
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("update")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("install")
        .arg("-y")
        .arg("libgpg-error-dev")
        .arg("libgpgme-dev")
        .arg("pkg-config")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    // Nothing should be cached on the first build.
    let repo_root = pwd.join(repo);
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    assert!(
        !messages.is_empty(),
        "build should produce cargo messages (this likely means --message-format is missing)"
    );

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
    pretty_assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    // We test that we can actually run the binary because the test cases
    // contain dynamically linked native libraries.
    Command::new()
        .pwd(&pwd)
        .name(repo_root.join("target").join("debug").join(bin))
        .arg("--help")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

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
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo2)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    assert!(
        !messages.is_empty(),
        "build should produce cargo messages (this likely means --message-format is missing)"
    );

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
    pretty_assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    // And we should still be able to run the binary.
    Command::new()
        .pwd(&pwd)
        .name(repo2.join("target").join("debug").join(bin))
        .arg("--help")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
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
    color_eyre::install()?;

    // Check for GITHUB_TOKEN early to fail fast with a clear error message
    if std::env::var("GITHUB_TOKEN").is_err() {
        bail!(
            "GITHUB_TOKEN environment variable is required to clone repositories from GitHub. \
             Please set it to a personal access token with 'repo' scope."
        );
    }

    // Start test environment with courier
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/workspace");

    // Install native dependencies required for building with native libs
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("update")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("install")
        .arg("-y")
        .arg("libgpg-error-dev")
        .arg("libgpgme-dev")
        .arg("pkg-config")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    // Nothing should be cached on the first build.
    let repo_root = pwd.join(repo);
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages = Build::new()
        .pwd(&repo_root)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    assert!(
        !messages.is_empty(),
        "build should produce cargo messages (this likely means --message-format is missing)"
    );

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
    pretty_assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh: {messages:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    // We test that we can actually run the binary because the test cases
    // contain dynamically linked native libraries.
    Command::new()
        .pwd(&pwd)
        .name(repo_root.join("target").join("debug").join(bin))
        .arg("--help")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
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
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    // Now if we clone the repo to a new directory and rebuild, `hurry` should
    // reuse the cache, which theoretically would enable fresh artifacts...
    let repo2 = pwd.join(format!("{repo}-2"));
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .dir(&repo2)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    // ... but since we uninstalled the native dependencies, the build should
    // actually fail to compile.
    let build = Build::new()
        .pwd(&repo2)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await;
    assert!(build.is_err(), "build should fail: {build:?}");

    Ok(())
}

/// Exercises building and caching the project across containers with a shared
/// cache (courier).
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[test_log::test(tokio::test)]
async fn cross_container(username: &str, repo: &str, branch: &str) -> Result<()> {
    color_eyre::install()?;

    // Check for GITHUB_TOKEN early to fail fast with a clear error message
    if std::env::var("GITHUB_TOKEN").is_err() {
        bail!(
            "GITHUB_TOKEN environment variable is required to clone repositories from GitHub. \
             Please set it to a personal access token with 'repo' scope."
        );
    }

    // Start test environment with courier
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/workspace");

    // We make the directories in which the project is cloned different in
    // each container to ensure nothing is accidentally getting reused via
    // filesystem; the cache sharing happens through courier.
    let pwd_repo_a = pwd.join(format!("{repo}-container-a"));
    let pwd_repo_b = pwd.join(format!("{repo}-container-b"));

    // Nothing should be cached on the first build in container A.
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .dir(&pwd_repo_a)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;
    let messages_a = Build::new()
        .pwd(&pwd_repo_a)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    assert!(
        !messages_a.is_empty(),
        "build should produce cargo messages (this likely means --message-format is missing)"
    );

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
    pretty_assert_eq!(
        expected,
        freshness,
        "no artifacts should be fresh in container A: {messages_a:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    // Now if we build in a different container with the same courier cache,
    // `hurry` should reuse the cache and enable fresh artifacts.
    Command::clone_github()
        .pwd(&pwd)
        .user(username)
        .repo(repo)
        .branch(branch)
        .dir(&pwd_repo_b)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_2)?)
        .await?;
    let messages_b = Build::new()
        .pwd(&pwd_repo_b)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_2)?)
        .await?;

    assert!(
        !messages_b.is_empty(),
        "build should produce cargo messages (this likely means --message-format is missing)"
    );

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
    pretty_assert_eq!(
        expected,
        freshness,
        "all artifacts should be fresh in container B: {messages_b:?}"
    );
    assert!(
        !expected.is_empty(),
        "build should have third-party artifacts"
    );

    Ok(())
}

/// Exercises building and caching the project concurrently across containers
/// with shared cache (courier). This test verifies that courier and hurry
/// handle concurrent builds correctly without corruption.
///
/// Important distinction: this test validates that the cache being shared and
/// built concurrently doesn't result in any corruption or failed builds. The
/// current design of `hurry` only checks and restores from the cache at the
/// very beginning of the build so it does not benefit from running builds
/// concurrently.
#[test_case("attunehq", "hurry-tests", "test/tiny"; "attunehq/hurry-tests:test/tiny")]
#[cfg_attr(feature = "ci", test_case("attunehq", "attune", "main"; "attunehq/attune:main"))]
#[cfg_attr(feature = "ci", test_case("attunehq", "hurry", "main"; "attunehq/hurry:main"))]
#[ignore = "This test has issues with mtime not matching up, which we've seen in actual real world use and should be solved by PR 212."]
#[test_log::test(tokio::test)]
async fn cross_container_concurrent(username: &str, repo: &str, branch: &str) -> Result<()> {
    color_eyre::install()?;

    // Check for GITHUB_TOKEN early to fail fast with a clear error message
    if std::env::var("GITHUB_TOKEN").is_err() {
        bail!(
            "GITHUB_TOKEN environment variable is required to clone repositories from GitHub. \
             Please set it to a personal access token with 'repo' scope."
        );
    }

    // Start test environment with courier
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/workspace");

    // We make the directories in which the project is cloned different in
    // each container to ensure nothing is accidentally getting reused via
    // filesystem; the cache sharing happens through courier.
    let pwd_repo_a = pwd.join(format!("{repo}-concurrent-a"));
    let pwd_repo_b = pwd.join(format!("{repo}-concurrent-b"));

    // Get container IDs upfront for use in concurrent operations
    let container_1 = env.service(TestEnv::HURRY_INSTANCE_1)?;
    let container_2 = env.service(TestEnv::HURRY_INSTANCE_2)?;

    // Clone repos in both containers concurrently
    tokio::try_join!(
        Command::clone_github()
            .pwd(&pwd)
            .user(username)
            .repo(repo)
            .branch(branch)
            .dir(&pwd_repo_a)
            .finish()
            .run_compose(&container_1),
        Command::clone_github()
            .pwd(&pwd)
            .user(username)
            .repo(repo)
            .branch(branch)
            .dir(&pwd_repo_b)
            .finish()
            .run_compose(&container_2)
    )?;

    // This is the main part of the test: both containers should be able to
    // build simultaneously without corrupting the shared cache (courier).
    let build_a = Build::new()
        .pwd(&pwd_repo_a)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish();
    let build_b = Build::new()
        .pwd(&pwd_repo_b)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish();
    let (messages_a, messages_b) = tokio::try_join!(
        build_a.run_compose(&container_1),
        build_b.run_compose(&container_2)
    )?;

    assert!(
        !messages_a.is_empty(),
        "build A should produce cargo messages (this likely means --message-format is missing)"
    );
    assert!(
        !messages_b.is_empty(),
        "build B should produce cargo messages (this likely means --message-format is missing)"
    );

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
    pretty_assert_eq!(
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
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish();
    let build_b = Build::new()
        .pwd(&pwd_repo_b)
        .wrapper(Build::HURRY_NAME)
        .api_url(env.api_url())
        .api_token(env.test_token())
        .finish();
    let (messages_a, messages_b) = tokio::try_join!(
        build_a.run_compose(&container_1),
        build_b.run_compose(&container_2)
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
    pretty_assert_eq!(
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
    pretty_assert_eq!(
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
    pretty_assert_eq!(
        expected_b,
        freshness_b,
        "all artifacts should be fresh: {messages_b:?}"
    );

    Ok(())
}
