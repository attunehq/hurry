use std::{fs::File, process::Command};

use color_eyre::{Result, eyre::Context};
use fslock::LockFile;
use testcontainers::compose::DockerCompose;

/// Test environment with ephemeral Docker Compose stack (Postgres + Courier).
///
/// This environment is fully isolated and cleaned up automatically via Drop.
/// Each test can create its own TestEnv without interfering with other tests.
pub struct TestEnv {
    compose: DockerCompose,
}

impl TestEnv {
    /// Ensure Docker Compose images are built.
    ///
    /// Uses file-based locking to coordinate builds across multiple test processes.
    /// Only builds images once, even when tests run in parallel via cargo nextest.
    ///
    /// This should be called before `new()` to avoid redundant builds when running
    /// tests in parallel.
    pub async fn ensure_built() -> Result<()> {
        let workspace_root = workspace_root::get_workspace_root();
        let compose_file = workspace_root.join("docker-compose.e2e.yml");

        // Get working tree hash to include uncommitted changes
        let hash = crate::container::working_tree_hash(&workspace_root)?;

        // Create marker and lock files in target directory with hash suffix
        let target_dir = workspace_root.join("target");
        let marker_file = target_dir.join(format!(".docker-compose-e2e_{hash}.built"));
        let lock_file_path = target_dir.join(".docker-compose-e2e.lock");

        // Fast path: check if already built for this hash
        if marker_file.exists() {
            tracing::debug!("docker compose images already built for hash {hash}");
            return Ok(());
        }

        // Acquire exclusive lock
        tracing::info!("acquiring lock for docker compose build...");
        let mut lock = LockFile::open(&lock_file_path)
            .context("open lock file for docker compose build")?;
        lock.lock().context("acquire lock for docker compose build")?;

        // Double-check after acquiring lock (another process might have built while we waited)
        if marker_file.exists() {
            tracing::debug!("docker compose images already built for hash {hash} (built by another process)");
            return Ok(());
        }

        // Build images using docker compose
        tracing::info!("building docker compose images for hash {hash}...");
        let status = Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(&compose_file)
            .arg("build")
            .status()
            .context("execute docker compose build")?;

        if !status.success() {
            color_eyre::eyre::bail!(
                "docker compose build failed with exit code: {}",
                status.code().unwrap_or(-1)
            );
        }

        // Create marker file for this hash
        File::create(&marker_file).context("create marker file for docker compose build")?;
        tracing::info!("docker compose images built successfully for hash {hash}");

        Ok(())
    }

    /// Create a new test environment.
    ///
    /// This will:
    /// - Start a Docker Compose stack with Postgres, migrations, fixtures, and Courier
    /// - Wait for all services to be healthy (using Docker Compose's built-in health checks)
    /// - Return once all services are ready
    ///
    /// The entire stack is automatically cleaned up when TestEnv is dropped.
    ///
    /// **Tip**: Call `TestEnv::ensure_built().await?` before this to coordinate image
    /// builds across parallel tests.
    pub async fn new() -> Result<Self> {
        tracing::info!("starting docker compose stack...");

        // Get workspace root and construct path to compose file
        let workspace_root = workspace_root::get_workspace_root();
        let compose_file = workspace_root.join("docker-compose.e2e.yml");
        let compose_file_str = compose_file
            .to_str()
            .ok_or_else(|| color_eyre::eyre::eyre!("invalid compose file path"))?;

        // Start compose stack (images should already be built via ensure_built)
        let mut compose = DockerCompose::with_local_client(&[compose_file_str]);
        compose.up().await?; // Waits for health checks automatically

        tracing::info!("docker compose stack ready");

        Ok(TestEnv { compose })
    }

    /// Get the URL to access Courier from the host machine.
    ///
    /// This returns the host-mapped port (e.g., "http://localhost:54321")
    pub async fn courier_url(&self) -> Result<String> {
        let courier = self
            .compose
            .service("courier")
            .ok_or_else(|| color_eyre::eyre::eyre!("courier service not found"))?;

        let port = courier.get_host_port_ipv4(3000).await?;
        Ok(format!("http://localhost:{port}"))
    }

    /// Get the test API token for authentication.
    ///
    /// This token is pre-loaded from the auth fixtures:
    /// - Token: `acme-alice-token-001`
    /// - Organization: Acme Corp
    /// - Account: alice@acme.com
    pub fn test_token(&self) -> &str {
        "acme-alice-token-001"
    }
}
