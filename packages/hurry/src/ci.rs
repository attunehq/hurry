//! CI environment detection.
//!
//! This module provides functionality to detect if the current process is
//! running in a Continuous Integration (CI) environment. This is useful for
//! adjusting behavior like waiting for uploads to complete (since CI daemons
//! won't persist).

use std::env;

/// Checks if an environment variable is set to a truthy value.
///
/// Truthy values are: "true" or "1"
///
/// Most CI providers use "true", but some use "1" as a boolean representation.
fn is_env_truthy(var: &str) -> bool {
    env::var(var).is_ok_and(|v| v == "true" || v == "1")
}

/// Detects if the current process is running in a CI environment.
///
/// Detection is based on standard environment variables set by CI providers:
/// - `CI=true` or `CI=1`: Set by GitHub Actions, GitLab CI, CircleCI, and many others
/// - Provider-specific variables for explicit detection
///
/// Reference: <https://github.com/semantic-release/env-ci>
/// This detection strategy is based on the widely-used env-ci library which
/// supports 32+ CI providers and uses CI=true as the standard detection method
///
/// # Examples
///
/// ```
/// use hurry::ci::is_ci;
///
/// if is_ci() {
///     println!("Running in CI environment");
/// }
/// ```
pub fn is_ci() -> bool {
    // Primary detection: Most CI providers set CI=true or CI=1
    if is_env_truthy("CI") {
        return true;
    }

    // Secondary detection: Provider-specific variables
    // GitHub Actions
    if is_env_truthy("GITHUB_ACTIONS") {
        return true;
    }

    // GitLab CI
    if is_env_truthy("GITLAB_CI") {
        return true;
    }

    // CircleCI
    if is_env_truthy("CIRCLECI") {
        return true;
    }

    // Jenkins
    if env::var("JENKINS_URL").is_ok() {
        return true;
    }

    // Travis CI
    if is_env_truthy("TRAVIS") {
        return true;
    }

    // Buildkite
    if is_env_truthy("BUILDKITE") {
        return true;
    }

    false
}
