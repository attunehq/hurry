use std::time::Duration;

use bollard::container::LogOutput;
use color_eyre::{Result, eyre::Context};
use futures::TryStreamExt;
use uuid::Uuid;

use crate::{Command, Container, Network};

/// Test environment with ephemeral Docker network, Postgres, and Courier.
///
/// This environment is fully isolated and cleaned up automatically via Drop.
/// Each test can create its own TestEnv without interfering with other tests.
pub struct TestEnv {
    /// Unique identifier for this test environment instance
    #[allow(dead_code)]
    id: String,
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
    /// - Run database migrations
    /// - Load authentication fixtures (test organizations, accounts, API keys)
    pub async fn new() -> Result<Self> {
        // Generate unique ID for this test environment instance
        let id = Uuid::new_v4().to_string();

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
        let postgres = start_postgres(&network, &id).await?;

        // Wait for Postgres to be ready
        tracing::info!("waiting for postgres to be ready...");
        wait_for_postgres(&postgres, &network).await?;
        tracing::info!("postgres is ready");

        // Run migrations
        tracing::info!("running migrations...");
        run_migrations(&network, &image_tag, &id).await?;
        tracing::info!("migrations complete");

        // Load test fixtures
        tracing::info!("loading fixtures...");
        load_auth_fixtures(&network, &id).await?;
        tracing::info!("fixtures loaded");

        Ok(TestEnv { id, network, postgres })
    }

}

/// Start a Postgres container on the given network.
///
/// The container is configured with:
/// - User: courier
/// - Password: courier
/// - Database: courier
/// - Container name: "postgres-{id}" (for DNS resolution within the network)
async fn start_postgres(network: &Network, id: &str) -> Result<Container> {
    Container::new()
        .repo("docker.io/library/postgres")
        .tag("18")
        .env("POSTGRES_USER", "courier")
        .env("POSTGRES_PASSWORD", "courier")
        .env("POSTGRES_DB", "courier")
        .env("POSTGRES_HOST_AUTH_METHOD", "trust")
        .network(network.id())
        .container_name(format!("postgres-{id}"))
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
async fn run_migrations(network: &Network, courier_image_tag: &str, id: &str) -> Result<()> {
    let (repo, tag) = courier_image_tag
        .split_once(':')
        .ok_or_else(|| color_eyre::eyre::eyre!("invalid image tag format"))?;

    let postgres_host = format!("postgres-{id}");
    let database_url = format!("postgres://courier:courier@{postgres_host}:5432/courier");

    tracing::info!("starting migration container with database_url: {database_url}");

    // Start courier container with migrate command as entrypoint
    let migrate_container = Container::new()
        .repo(repo)
        .tag(tag)
        .network(network.id())
        .container_name(format!("migrate-{id}"))
        .entrypoint(["migrate", "--database-url", &database_url])
        .start()
        .await
        .context("start migration container")?;

    tracing::info!("migration container started, waiting for completion...");

    // Wait for the migration container to exit (with timeout)
    let mut wait_stream = migrate_container
        .docker()
        .wait_container(migrate_container.id(), None::<bollard::query_parameters::WaitContainerOptions>);

    // Get the exit status with a 30 second timeout
    let wait_response = tokio::time::timeout(Duration::from_secs(30), wait_stream.try_next())
        .await
        .context("migration container timed out after 30 seconds")?
        .context("wait for migration container")?
        .ok_or_else(|| color_eyre::eyre::eyre!("no wait response from migration container"))?;

    // Check that migrations succeeded
    if wait_response.status_code != 0 {
        // Get logs to help debug the failure
        let logs = migrate_container
            .docker()
            .logs(
                migrate_container.id(),
                Some(bollard::query_parameters::LogsOptionsBuilder::default().stdout(true).stderr(true).build()),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap_or_default();

        let log_output = logs
            .iter()
            .filter_map(|log| match log {
                LogOutput::StdOut { message } |
                LogOutput::StdErr { message } =>
                    String::from_utf8_lossy(message).into_owned().into(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        color_eyre::eyre::bail!(
            "migrations failed with exit code: {}\nLogs:\n{}",
            wait_response.status_code,
            log_output
        );
    }

    Ok(())
}

/// Load authentication fixtures from auth.sql into Postgres.
///
/// This creates an ephemeral container that runs psql to load the fixture file.
/// The fixtures provide test organizations, accounts, and API keys for testing.
async fn load_auth_fixtures(network: &Network, id: &str) -> Result<()> {
    let workspace_root = workspace_root::get_workspace_root();
    let fixtures_path = workspace_root.join("packages/courier/schema/fixtures/auth.sql");

    // Validate fixture file exists
    if !fixtures_path.exists() {
        color_eyre::eyre::bail!("auth fixtures file not found: {}", fixtures_path.display());
    }

    let postgres_host = format!("postgres-{id}");

    // Start postgres container with psql command to load fixtures
    // Mount the fixtures file into the container and execute it
    let fixtures_container = Container::new()
        .repo("docker.io/library/postgres")
        .tag("18")
        .network(network.id())
        .container_name(format!("fixtures-{id}"))
        .volume_bind(&fixtures_path, "/tmp/auth.sql")
        .entrypoint([
            "psql",
            "-h",
            &postgres_host,
            "-U",
            "courier",
            "-d",
            "courier",
            "-f",
            "/tmp/auth.sql",
        ])
        .start()
        .await
        .context("start fixtures container")?;

    // Wait for the fixtures container to exit
    let mut wait_stream = fixtures_container
        .docker()
        .wait_container(fixtures_container.id(), None::<bollard::query_parameters::WaitContainerOptions>);

    // Get the exit status
    let wait_response = wait_stream
        .try_next()
        .await
        .context("wait for fixtures container")?
        .ok_or_else(|| color_eyre::eyre::eyre!("no wait response from fixtures container"))?;

    // Check that fixtures loaded successfully
    if wait_response.status_code != 0 {
        color_eyre::eyre::bail!(
            "loading fixtures failed with exit code: {}",
            wait_response.status_code
        );
    }

    Ok(())
}
