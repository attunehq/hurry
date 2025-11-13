use std::time::Duration;

use color_eyre::{Result, eyre::Context};
use futures::TryStreamExt;

use crate::{Command, Container, Network};

/// Test environment with ephemeral Docker network, Postgres, and Courier.
///
/// This environment is fully isolated and cleaned up automatically via Drop.
/// Each test can create its own TestEnv without interfering with other tests.
pub struct TestEnv {
    pub network: Network,
    pub postgres: Container,
}

impl TestEnv {
    /// Create a new test environment.
    ///
    /// This will:
    /// - Build the courier Docker image (cached after first build)
    /// - Create an isolated Docker network
    /// - Start a Postgres container
    /// - Wait for Postgres to be ready
    pub async fn new() -> Result<Self> {
        // Build courier image (returns full tag like "hurry-courier:abc1234")
        let image_tag = Container::ensure_built(
            "hurry-courier",
            "docker/courier/Dockerfile",
            ".",
        )
        .await?;

        // Create isolated network
        let network = Network::create().await?;

        // Start Postgres
        let postgres = start_postgres(&network).await?;

        // Wait for Postgres to be ready
        wait_for_postgres(&postgres, &network).await?;

        // Run migrations
        run_migrations(&network, &image_tag).await?;

        Ok(TestEnv { network, postgres })
    }

}

/// Start a Postgres container on the given network.
///
/// The container is configured with:
/// - User: courier
/// - Password: courier
/// - Database: courier
/// - Container name: "postgres" (for DNS resolution)
async fn start_postgres(network: &Network) -> Result<Container> {
    Container::new()
        .repo("docker.io/library/postgres")
        .tag("18")
        .env("POSTGRES_USER", "courier")
        .env("POSTGRES_PASSWORD", "courier")
        .env("POSTGRES_DB", "courier")
        .env("POSTGRES_HOST_AUTH_METHOD", "trust")
        .network(network.id())
        .container_name("postgres")
        .start()
        .await
        .context("start postgres container")
}

/// Wait for Postgres to be ready to accept connections.
///
/// This function polls Postgres using `pg_isready` command from within the
/// Docker network. It retries for up to 30 seconds with a simple delay strategy.
async fn wait_for_postgres(postgres: &Container, _network: &Network) -> Result<()> {
    let timeout = Duration::from_secs(30);
    let start = std::time::Instant::now();

    // Give postgres a moment to start up before we begin checking
    tokio::time::sleep(Duration::from_secs(2)).await;

    loop {
        // Check if we've exceeded the timeout
        if start.elapsed() >= timeout {
            color_eyre::eyre::bail!("timeout waiting for postgres to be ready");
        }

        // Run pg_isready directly in the postgres container
        let check_result = Command::new()
            .pwd("/")
            .name("pg_isready")
            .arg("-U")
            .arg("courier")
            .finish()
            .run_docker(postgres)
            .await;

        match check_result {
            Ok(()) => {
                // pg_isready succeeded, postgres is ready
                return Ok(());
            }
            Err(_) => {
                // pg_isready failed, postgres not ready yet, retry
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

/// Run database migrations using the courier binary.
///
/// This creates an ephemeral container that runs `courier migrate` and waits
/// for it to complete. The container exits after migrations are applied.
async fn run_migrations(network: &Network, courier_image_tag: &str) -> Result<()> {
    let (repo, tag) = courier_image_tag
        .split_once(':')
        .ok_or_else(|| color_eyre::eyre::eyre!("invalid image tag format"))?;

    // Start courier container with migrate command as entrypoint
    let migrate_container = Container::new()
        .repo(repo)
        .tag(tag)
        .network(network.id())
        .entrypoint([
            "migrate",
            "--database-url",
            "postgres://courier:courier@postgres:5432/courier",
        ])
        .start()
        .await
        .context("start migration container")?;

    // Wait for the migration container to exit
    let mut wait_stream = migrate_container
        .docker()
        .wait_container(migrate_container.id(), None::<bollard::query_parameters::WaitContainerOptions>);

    // Get the exit status
    let wait_response = wait_stream
        .try_next()
        .await
        .context("wait for migration container")?
        .ok_or_else(|| color_eyre::eyre::eyre!("no wait response from migration container"))?;

    // Check that migrations succeeded
    if wait_response.status_code != 0 {
        color_eyre::eyre::bail!(
            "migrations failed with exit code: {}",
            wait_response.status_code
        );
    }

    Ok(())
}
