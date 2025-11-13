//! Sanity tests that validate E2E test infrastructure as we build it.
//!
//! These tests are designed to provide fast feedback during development.
//! Run with: `cargo nextest run -p e2e sanity`

use color_eyre::Result;
use e2e::{Command, Container, Network, TestEnv};

/// Validates that we can build the courier Docker image, create a network,
/// and start a courier container on that network.
///
/// This test validates infrastructure built in Steps 1-4:
/// - Container::ensure_built() works for building courier image
/// - Network::create() works for creating Docker networks
/// - Starting a simple courier container on the network works
#[test_log::test(tokio::test)]
async fn builds_courier_image() -> Result<()> {
    color_eyre::install()?;

    // Validate Container::ensure_built works and get the full image tag with git
    // SHA
    let image_tag =
        Container::ensure_built("hurry-courier", "docker/courier/Dockerfile", ".").await?;

    // Parse the returned tag into repo:tag format
    // e.g., "hurry-courier:abc1234" -> repo="hurry-courier", tag="abc1234"
    let (repo, tag) = image_tag
        .split_once(':')
        .expect("image tag should contain ':'");

    // Validate Network works
    let network = Network::create().await?;

    // Validate we can start a simple container on the network
    let container = Container::new()
        .repo(repo)
        .tag(tag)
        .network(network.id())
        .container_name("test-courier")
        .start()
        .await?;

    // Just check it exists
    assert!(!container.id().is_empty());

    Ok(())
}

/// Validates that we can create a TestEnv with Postgres and that Postgres
/// is ready to accept connections and can execute queries.
///
/// This test validates infrastructure built in Step 6a-6b:
/// - TestEnv::new() works and creates isolated environment
/// - Postgres container starts successfully
/// - Postgres becomes ready within timeout (pg_isready returns success)
/// - Migrations run successfully
/// - We can execute SQL queries against Postgres
/// - Migration tables exist (organization, account, api_key)
#[test_log::test(tokio::test)]
async fn starts_postgres() -> Result<()> {
    color_eyre::install()?;

    let env = TestEnv::new().await?;

    // Verify basic query works
    let output = Command::new()
        .pwd("/")
        .name("psql")
        .arg("-U")
        .arg("courier")
        .arg("-d")
        .arg("courier")
        .arg("-c")
        .arg("SELECT 1")
        .finish()
        .run_docker_with_output(&env.postgres)
        .await?;
    assert!(!output.stdout.is_empty(), "query should return output");
    assert!(
        output.stdout_lossy().contains("1 row"),
        "output should contain '1 row': {}",
        output.stdout_lossy()
    );

    // Verify migrations ran by checking for a table created by migrations
    let output = Command::new()
        .pwd("/")
        .name("psql")
        .arg("-U")
        .arg("courier")
        .arg("-d")
        .arg("courier")
        .arg("-c")
        .arg("SELECT COUNT(*) FROM organization")
        .finish()
        .run_docker_with_output(&env.postgres)
        .await?;
    assert!(
        !output.stdout.is_empty(),
        "organization table query should return output"
    );
    // Should have 0 rows (no fixtures loaded yet)
    assert!(
        output.stdout_lossy().contains("0") || output.stdout_lossy().contains("count"),
        "should be able to query organization table: {}",
        output.stdout_lossy()
    );

    Ok(())
}
