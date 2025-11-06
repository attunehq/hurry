//! CI environment detection.
//!
//! This module provides functionality to detect if the current process is
//! running in a Continuous Integration (CI) environment. This is useful for
//! adjusting behavior like waiting for uploads to complete (since CI daemons
//! won't persist).

use std::env;

/// CI environment variable detection patterns.
///
/// Each entry specifies how to detect a specific CI provider.
enum CiCheckVar {
    /// Check if the variable exists and equals "true" or "1"
    Truthy(&'static str),
    /// Check if the variable exists (any value)
    Present(&'static str),
    /// Check if the variable exists and equals a specific value
    Equals(&'static str, &'static str),
}

/// List of CI environment variables to check for CI detection.
///
/// This list is based on the env-ci library (<https://github.com/semantic-release/env-ci>).
/// Variables are checked in order, with the generic `CI` variable first (set by
/// most providers), followed by provider-specific variables for explicit
/// detection.
const CI_VARS: &[CiCheckVar] = &[
    // Generic CI variable: Set by most CI providers
    CiCheckVar::Truthy("CI"),
    // Provider-specific variables (alphabetically ordered)
    CiCheckVar::Truthy("APPVEYOR"),                       // Appveyor
    CiCheckVar::Present("BUILD_BUILDURI"),                // Azure Pipelines
    CiCheckVar::Present("bamboo_agentId"),                // Bamboo
    CiCheckVar::Present("BITBUCKET_BUILD_NUMBER"),        // Bitbucket Pipelines
    CiCheckVar::Truthy("BITRISE_IO"),                     // Bitrise
    CiCheckVar::Present("BUDDY_WORKSPACE_ID"),            // Buddy
    CiCheckVar::Truthy("BUILDKITE"),                      // Buildkite
    CiCheckVar::Equals("CF_PAGES", "1"),                  // Cloudflare Pages
    CiCheckVar::Present("CF_BUILD_ID"),                   // Codefresh
    CiCheckVar::Truthy("CIRCLECI"),                       // CircleCI
    CiCheckVar::Truthy("CIRRUS_CI"),                      // Cirrus CI
    CiCheckVar::Equals("CI_NAME", "codeship"),            // Codeship
    CiCheckVar::Present("CODEBUILD_BUILD_ID"),            // AWS CodeBuild
    CiCheckVar::Present("DISTELLI_APPNAME"),              // Puppet (Distelli)
    CiCheckVar::Truthy("DRONE"),                          // Drone
    CiCheckVar::Truthy("GITHUB_ACTIONS"),                 // GitHub Actions
    CiCheckVar::Truthy("GITLAB_CI"),                      // GitLab CI
    CiCheckVar::Present("JB_SPACE_EXECUTION_NUMBER"),     // JetBrains Space
    CiCheckVar::Present("JENKINS_URL"),                   // Jenkins
    CiCheckVar::Equals("NETLIFY", "true"),                // Netlify
    CiCheckVar::Present("NOW_GITHUB_DEPLOYMENT"),         // Vercel (legacy Zeit Now)
    CiCheckVar::Truthy("SAILCI"),                         // Sail CI
    CiCheckVar::Truthy("SCREWDRIVER"),                    // Screwdriver.cd
    CiCheckVar::Truthy("SCRUTINIZER"),                    // Scrutinizer
    CiCheckVar::Truthy("SEMAPHORE"),                      // Semaphore
    CiCheckVar::Truthy("SHIPPABLE"),                      // Shippable
    CiCheckVar::Present("TEAMCITY_VERSION"),              // TeamCity
    CiCheckVar::Truthy("TRAVIS"),                         // Travis CI
    CiCheckVar::Truthy("VELA"),                           // Vela
    CiCheckVar::Truthy("VERCEL"),                         // Vercel
    CiCheckVar::Present("WERCKER_MAIN_PIPELINE_STARTED"), // Wercker
];

/// Checks if an environment variable matches the given CI detection pattern.
fn matches_ci_var(ci_var: &CiCheckVar) -> bool {
    match ci_var {
        CiCheckVar::Truthy(var) => env::var(var).is_ok_and(|v| v == "true" || v == "1"),
        CiCheckVar::Present(var) => env::var(var).is_ok(),
        CiCheckVar::Equals(var, expected) => env::var(var).is_ok_and(|v| v == *expected),
    }
}

/// Detects if the current process is running in a CI environment.
///
/// Detection is based on environment variables set by CI providers:
/// - `CI=true` or `CI=1`: Set by GitHub Actions, GitLab CI, CircleCI, and many
///   others
/// - Provider-specific variables for explicit detection
///
/// Reference: <https://github.com/semantic-release/env-ci>
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
    CI_VARS.iter().any(matches_ci_var)
}
