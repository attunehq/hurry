//! Sanity tests that validate E2E test infrastructure.
//!
//! These tests are designed to provide fast feedback during development.
//! Run with: `cargo nextest run -p e2e sanity`

use color_eyre::Result;
use e2e::{Command, TestEnv};

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

    // Ensure images are built (with cross-process coordination)
    TestEnv::ensure_built().await?;

    // Start the ephemeral test environment
    let env = TestEnv::new().await?;

    // Verify we can get the courier URL (requires service to be running)
    let courier_url = env.courier_url().await?;
    assert!(
        courier_url.starts_with("http://localhost:"),
        "courier URL should be host-mapped: {courier_url}"
    );

    // Verify we can get the test token
    let token = env.test_token();
    assert_eq!(token, "acme-alice-token-001");

    // Verify courier health endpoint responds
    let health_url = format!("{courier_url}/api/v1/health");
    let response = reqwest::get(&health_url).await?;
    assert!(
        response.status().is_success(),
        "health check should succeed, got status: {}",
        response.status()
    );

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

    // Ensure images are built
    TestEnv::ensure_built().await?;

    // Start the test environment
    let env = TestEnv::new().await?;

    // Run a simple command to verify hurry is installed
    Command::new()
        .name("hurry")
        .arg("--version")
        .pwd("/workspace")
        .finish()
        .run_compose(env.hurry_container_id())
        .await?;

    Ok(())
}
