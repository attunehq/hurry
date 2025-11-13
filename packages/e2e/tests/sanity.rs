//! Sanity tests that validate E2E test infrastructure as we build it.
//!
//! These tests are designed to provide fast feedback during development.
//! Run with: `cargo nextest run -p e2e sanity`

use color_eyre::Result;
use e2e::{Container, Network};

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

    // Validate Container::ensure_built works and get the full image tag with git SHA
    let image_tag = Container::ensure_built(
        "hurry-courier",
        "docker/courier/Dockerfile",
        ".",
    ).await?;

    // Parse the returned tag into repo:tag format
    // e.g., "hurry-courier:test-abc1234" -> repo="hurry-courier", tag="test-abc1234"
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
