use std::{fs::File, process::Command};

use color_eyre::{
    Result,
    eyre::{Context, bail, eyre},
};
use fslock::LockFile;
use testcontainers::compose::DockerCompose;
use tracing::{debug, info, instrument};
use workspace_root::get_workspace_root;

/// Test environment with ephemeral Docker Compose stack (Postgres + Courier +
/// Hurry).
///
/// This environment is fully isolated and cleaned up automatically via Drop.
/// Each test can create its own TestEnv without interfering with other tests.
///
/// ## Multi-container support
///
/// The compose stack includes two hurry containers (`hurry-1` and `hurry-2`) to
/// support tests that need multiple isolated containers (e.g., testing cache
/// sharing across containers). Access them via [`TestEnv::service`] with
/// service names like [`TestEnv::HURRY_INSTANCE_1`] and
/// [`TestEnv::HURRY_INSTANCE_2`].
///
/// Both containers:
/// - Use the same debian-rust image with hurry installed
/// - Share the compose network (can communicate with courier/postgres)
/// - Are fully isolated from other parallel tests (each TestEnv gets its own
///   stack)
///
/// Single-container tests should use [`TestEnv::HURRY_INSTANCE_1`].
pub struct TestEnv {
    compose: DockerCompose,
}

impl TestEnv {
    /// Service name for the first hurry container instance.
    pub const HURRY_INSTANCE_1: &str = "hurry-1";

    /// Service name for the second hurry container instance.
    pub const HURRY_INSTANCE_2: &str = "hurry-2";

    /// Ensure Docker Compose images are built.
    ///
    /// Uses file-based locking to coordinate builds across multiple test
    /// processes. Only builds images once, even when tests run in parallel
    /// via cargo nextest.
    #[instrument]
    async fn ensure_built() -> Result<()> {
        let workspace_root = get_workspace_root();
        let compose_file = workspace_root.join("docker-compose.e2e.yml");

        // Get working tree hash to include uncommitted changes
        let hash = crate::container::working_tree_hash(&workspace_root)?;

        // Create marker and lock files in target directory with hash suffix
        let target_dir = workspace_root.join("target");
        let marker_file = target_dir.join(format!(".docker-compose-e2e_{hash}.built"));
        let build_lockfile = target_dir.join(".docker-compose-e2e.lock");

        // Fast path: check if already built for this hash.
        // The marker file isn't created until after the build finishes.
        if marker_file.exists() {
            debug!("docker compose images already built for hash {hash}");
            return Ok(());
        }

        // Acquire exclusive lock
        info!("acquiring lock for docker compose build...");
        let mut build = LockFile::open(&build_lockfile).context("open docker build lockfile")?;
        build.lock().context("lock docker build")?;

        // Another process may have built while we were waiting.
        if marker_file.exists() {
            debug!("docker compose images already built for hash {hash}");
            return Ok(());
        }

        info!("building docker compose images for hash {hash}...");
        let status = Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(&compose_file)
            .arg("build")
            .status()
            .context("execute docker compose build")?;
        if !status.success() {
            bail!(
                "docker compose build failed with exit code: {}",
                status.code().unwrap_or(-1)
            );
        }

        // Create marker file for this hash now that the images are built.
        File::create(&marker_file).context("create marker file for docker compose build")?;
        info!("docker compose images built successfully for hash {hash}");

        // And we're done! The lock is dropped when we exit.
        Ok(())
    }

    /// Create a new test environment.
    ///
    /// This will:
    /// - Build Docker Compose images if needed (coordinated across parallel
    ///   tests)
    /// - Start a Docker Compose stack with Postgres, migrations, fixtures, and
    ///   Courier
    /// - Wait for all services to be healthy (using Docker Compose's built-in
    ///   health checks)
    /// - Return once all services are ready
    ///
    /// The entire stack is automatically cleaned up when TestEnv is dropped.
    #[instrument]
    pub async fn new() -> Result<Self> {
        Self::ensure_built().await.context("build compose stack")?;

        // Get workspace root and construct path to compose file
        info!("starting docker compose stack...");
        let workspace_root = get_workspace_root();
        let compose_file = workspace_root.join("docker-compose.e2e.yml");
        let compose_file = compose_file
            .to_str()
            .ok_or_else(|| eyre!("invalid compose file path"))?;

        // Images were already built via `ensure_built`, so we can just start.
        let mut compose = DockerCompose::with_local_client(&[compose_file]);

        // The compose file has health checks built in, so we don't have to.
        compose.up().await?;

        info!("docker compose stack ready");
        Ok(TestEnv { compose })
    }

    /// Get the URL to access the Hurry API from within the Docker Compose
    /// network.
    ///
    /// Returns the internal service URL (e.g., "http://courier:3000") that
    /// containers can use to communicate with the Hurry API over the shared
    /// network.
    pub fn api_url(&self) -> String {
        String::from("http://courier:3000")
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

    /// Get the Docker container ID for a compose service.
    ///
    /// Returns the Docker container ID for the specified service name from the
    /// compose stack. Use this with `Command::run_compose()` to execute
    /// commands inside the container.
    ///
    /// # Arguments
    /// * `service_name` - The service name from docker-compose.e2e.yml
    ///
    /// # Returns
    /// The Docker container ID as a string.
    ///
    /// # Errors
    /// Returns an error if the service is not found in the compose stack.
    ///
    /// # Example
    /// ```ignore
    /// let env = TestEnv::new().await?;
    /// let container_id = env.service(TestEnv::HURRY_INSTANCE_1)?;
    /// Command::new()
    ///     .name("hurry")
    ///     .arg("--version")
    ///     .finish()
    ///     .run_compose(container_id)
    ///     .await?;
    /// ```
    pub fn service(&self, name: &str) -> Result<String> {
        self.compose
            .service(name)
            .ok_or_else(|| eyre!("service '{name}' not found in compose stack"))
            .map(|container| container.id().to_string())
    }
}
