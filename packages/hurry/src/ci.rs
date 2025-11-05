//! CI environment detection.
//!
//! This module provides functionality to detect if the current process is
//! running in a Continuous Integration (CI) environment. This is useful for
//! adjusting behavior like waiting for uploads to complete (since CI daemons
//! won't persist).

use std::env;

/// Detects if the current process is running in a CI environment.
///
/// Detection is based on standard environment variables set by CI providers:
/// - `CI=true`: Set by GitHub Actions, GitLab CI, CircleCI, and many others
/// - Provider-specific variables for explicit detection
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
    // Primary detection: Most CI providers set CI=true
    if env::var("CI").is_ok_and(|v| v == "true") {
        return true;
    }

    // Secondary detection: Provider-specific variables
    // GitHub Actions
    if env::var("GITHUB_ACTIONS").is_ok_and(|v| v == "true") {
        return true;
    }

    // GitLab CI
    if env::var("GITLAB_CI").is_ok_and(|v| v == "true") {
        return true;
    }

    // CircleCI
    if env::var("CIRCLECI").is_ok_and(|v| v == "true") {
        return true;
    }

    // Jenkins
    if env::var("JENKINS_URL").is_ok() {
        return true;
    }

    // Travis CI
    if env::var("TRAVIS").is_ok_and(|v| v == "true") {
        return true;
    }

    // Buildkite
    if env::var("BUILDKITE").is_ok_and(|v| v == "true") {
        return true;
    }

    false
}
