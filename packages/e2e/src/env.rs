use std::{fs::File, process::Command};

use color_eyre::{Result, eyre::Context};
use fslock::LockFile;
use testcontainers::compose::DockerCompose;

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
/// sharing across containers). Access them via `hurry_container_id(1)` and
/// `hurry_container_id(2)`.
///
/// Both containers:
/// - Use the same debian-rust image with hurry installed
/// - Share the compose network (can communicate with courier/postgres)
/// - Are fully isolated from other parallel tests (each TestEnv gets its own
///   stack)
///
/// Single-container tests should use `hurry_container_id(1)`.
pub struct TestEnv {
    #[allow(dead_code)]
    compose: DockerCompose,
    hurry_1_id: String,
    hurry_2_id: String,
}

impl TestEnv {
    /// Ensure Docker Compose images are built.
    ///
    /// Uses file-based locking to coordinate builds across multiple test
    /// processes. Only builds images once, even when tests run in parallel
    /// via cargo nextest.
    ///
    /// This should be called before `new()` to avoid redundant builds when
    /// running tests in parallel.
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
        let mut lock =
            LockFile::open(&lock_file_path).context("open lock file for docker compose build")?;
        lock.lock()
            .context("acquire lock for docker compose build")?;

        // Double-check after acquiring lock (another process might have built while we
        // waited)
        if marker_file.exists() {
            tracing::debug!(
                "docker compose images already built for hash {hash} (built by another process)"
            );
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
    /// - Start a Docker Compose stack with Postgres, migrations, fixtures, and
    ///   Courier
    /// - Wait for all services to be healthy (using Docker Compose's built-in
    ///   health checks)
    /// - Return once all services are ready
    ///
    /// The entire stack is automatically cleaned up when TestEnv is dropped.
    ///
    /// **Tip**: Call `TestEnv::ensure_built().await?` before this to coordinate
    /// image builds across parallel tests.
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

        let hurry_1_id = compose
            .service("hurry-1")
            .ok_or_else(|| color_eyre::eyre::eyre!("hurry-1 service not found"))?
            .id()
            .to_string();

        let hurry_2_id = compose
            .service("hurry-2")
            .ok_or_else(|| color_eyre::eyre::eyre!("hurry-2 service not found"))?
            .id()
            .to_string();

        Ok(TestEnv {
            compose,
            hurry_1_id,
            hurry_2_id,
        })
    }

    /// Get the URL to access Courier from within the Docker Compose network.
    ///
    /// Returns the internal service URL (e.g., "http://courier:3000") that
    /// containers can use to communicate with courier over the shared network.
    pub fn courier_url(&self) -> String {
        "http://courier:3000".to_string()
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

    /// Get a hurry container ID for running commands.
    ///
    /// Returns the Docker container ID of the specified hurry service (1 or 2).
    /// Each hurry container is a Debian-based container with Rust and hurry
    /// installed. Use this with `Command::run_compose()` to execute
    /// commands inside the container.
    ///
    /// The compose stack provides two hurry containers to support tests that
    /// need multiple isolated containers (e.g., testing cache sharing
    /// across containers).
    ///
    /// # Arguments
    /// * `index` - Which hurry container to use (1 or 2)
    ///
    /// # Panics
    /// Panics if index is not 1 or 2.
    ///
    /// # Example
    /// ```ignore
    /// let env = TestEnv::new().await?;
    /// Command::new()
    ///     .name("hurry")
    ///     .arg("--version")
    ///     .finish()
    ///     .run_compose(env.hurry_container_id(1))
    ///     .await?;
    /// ```
    pub fn hurry_container_id(&self, index: u8) -> &str {
        match index {
            1 => &self.hurry_1_id,
            2 => &self.hurry_2_id,
            _ => panic!("hurry container index must be 1 or 2, got {index}"),
        }
    }
}
