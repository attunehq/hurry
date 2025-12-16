//! Sanity tests that validate E2E test infrastructure.
//!
//! These tests are designed to provide fast feedback during development.
//! Run with: `cargo nextest run -p e2e sanity`

use color_eyre::Result;
use e2e::{Command, TestEnv};
use pretty_assertions::assert_eq as pretty_assert_eq;

/// Validates that the TestEnv Docker Compose stack starts successfully.
///
/// This test validates:
/// - Docker Compose images are built (coordinated across parallel tests)
/// - All services start (postgres, migrate, fixtures, courier)
/// - All health checks pass
/// - Courier service is accessible via host-mapped port
/// - Test authentication token is available
#[test_log::test(tokio::test)]
async fn compose_stack_starts() -> Result<()> {
    color_eyre::install()?;

    // Start the ephemeral test environment
    let env = TestEnv::new().await?;

    // Verify we can get the API URL (internal Docker network URL)
    let api_url = env.api_url();
    pretty_assert_eq!(api_url, "http://courier:3000");

    // Verify we can get the test token
    let token = env.test_token();
    pretty_assert_eq!(token, "acme-alice-token-001");

    Ok(())
}

/// Validates that the hurry container can execute commands.
///
/// This test validates:
/// - Hurry service container is running
/// - Commands can be executed in the hurry container via run_compose
/// - Hurry binary is installed and accessible
#[test_log::test(tokio::test)]
async fn hurry_container_runs_commands() -> Result<()> {
    color_eyre::install()?;

    // Start the test environment
    let env = TestEnv::new().await?;

    // Run a simple command to verify hurry is installed
    Command::new()
        .name("hurry")
        .arg("--version")
        .pwd("/workspace")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await?;

    Ok(())
}
