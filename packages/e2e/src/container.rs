use std::{collections::HashMap, path::{Path, PathBuf}, sync::Arc};

use bollard::{
    Docker,
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
        StartContainerOptionsBuilder,
    },
    secret::{ContainerCreateBody, EndpointSettings, HostConfig, NetworkingConfig},
};
use bon::bon;
use color_eyre::{
    Result,
    eyre::{Context, OptionExt, bail},
};
use fslock::LockFile;
use futures::TryStreamExt;

use crate::Command;

/// References a running Docker container.
///
/// This reference uses interior mutability to track internally track references
/// to the container. After the final reference is dropped, attempts to remove
/// the container from the docker daemon.
///
/// Attempts to remove the container from Docker when dropped, although since
/// there is no such thing as async drop yet this is best effort and will likely
/// lead to many orphan containers.
#[derive(Clone, Debug)]
pub struct Container {
    inner: Arc<ContainerRef>,
}

impl Container {
    /// Reference to the Docker client.
    pub fn docker(&self) -> &Docker {
        &self.inner.docker
    }

    /// The ID of the container running in the docker context.
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Ensure a Docker image is built from a Dockerfile.
    ///
    /// Uses file-based locking to coordinate builds across multiple test processes.
    /// Only builds the image once, even when tests run in parallel via cargo nextest.
    ///
    /// The image is automatically tagged with the current git commit SHA to ensure
    /// fresh images when code changes. For example, "hurry-courier" becomes "hurry-courier:abc1234".
    ///
    /// # Arguments
    /// - `image_name`: The name of the image (e.g., "hurry-courier")
    /// - `dockerfile`: Path to the Dockerfile relative to workspace root (e.g., "docker/courier/Dockerfile")
    /// - `context`: Build context directory relative to workspace root (typically ".")
    ///
    /// # Returns
    /// The full image tag including git SHA (e.g., "hurry-courier:abc1234")
    ///
    /// # Example
    /// ```ignore
    /// let full_tag = Container::ensure_built(
    ///     "hurry-courier",
    ///     "docker/courier/Dockerfile",
    ///     ".",
    /// ).await?;
    /// // full_tag is now "hurry-courier:abc1234"
    /// ```
    pub async fn ensure_built(
        image_name: impl AsRef<str>,
        dockerfile: impl AsRef<Path>,
        context: impl AsRef<Path>,
    ) -> Result<String> {
        let image_name = image_name.as_ref();
        let dockerfile = dockerfile.as_ref();
        let context = context.as_ref();

        let workspace_root = workspace_root::get_workspace_root();
        let target_dir = workspace_root.join("target");

        // Get a SHA representing the current working tree state (including uncommitted changes)
        // First, get the commit SHA
        let commit_sha = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&workspace_root)
            .output()
            .context("execute git rev-parse")?;

        if !commit_sha.status.success() {
            bail!("git rev-parse failed with status: {}", commit_sha.status);
        }

        let sha = String::from_utf8(commit_sha.stdout)
            .context("parse git SHA as UTF-8")?
            .trim()
            .to_string();

        // Check if there are any uncommitted changes (staged or unstaged)
        // Use git diff to get a hash of the actual content changes
        let git_diff = std::process::Command::new("git")
            .args(["diff", "HEAD"])
            .current_dir(&workspace_root)
            .output()
            .context("execute git diff")?;

        if !git_diff.status.success() {
            bail!("git diff failed with status: {}", git_diff.status);
        }

        // If there are uncommitted changes, create a unique hash by combining
        // the commit SHA with a hash of the diff
        let sha = if !git_diff.stdout.is_empty() {
            // Compute a hash of the diff output (captures actual content changes)
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            git_diff.stdout.hash(&mut hasher);
            let dirty_hash = hasher.finish();

            format!("{sha}-{dirty_hash:x}")
        } else {
            sha
        };

        // Build full image tag with git SHA
        let image_tag = format!("{image_name}:{sha}");

        let sanitized_tag = image_tag.replace(':', "_").replace('/', "_");
        let marker_path = target_dir.join(format!(".{sanitized_tag}.built"));
        let lock_path = target_dir.join(format!(".{sanitized_tag}.lock"));

        // Fast path: check if marker file exists
        if marker_path.exists() {
            return Ok(image_tag);
        }

        // Slow path: acquire lock and build if needed
        eprintln!("[BUILD] Waiting for exclusive lock to build {image_tag}...");
        let mut lock = LockFile::open(&lock_path)
            .with_context(|| format!("open lock file {lock_path:?}"))?;
        lock.lock()
            .with_context(|| format!("acquire lock for {image_tag}"))?;

        eprintln!("[BUILD] Lock acquired, checking if image needs building...");

        // Double-check marker after acquiring lock
        if marker_path.exists() {
            eprintln!("[BUILD] Image {image_tag} was built by another process");
            return Ok(image_tag);
        }

        // Build the image
        eprintln!("[BUILD] Building Docker image {image_tag}...");

        let dockerfile_path = workspace_root.join(dockerfile);
        let context_path = workspace_root.join(context);

        let status = std::process::Command::new("docker")
            .args([
                "build",
                "-t",
                &image_tag,
                "-f",
                dockerfile_path
                    .to_str()
                    .ok_or_else(|| color_eyre::eyre::eyre!("invalid dockerfile path"))?,
                context_path
                    .to_str()
                    .ok_or_else(|| color_eyre::eyre::eyre!("invalid context path"))?,
            ])
            .status()
            .context("execute docker build")?;

        if !status.success() {
            bail!("docker build failed with status: {status}");
        }

        eprintln!("[BUILD] Successfully built {image_tag}");

        // Create marker file
        std::fs::write(&marker_path, "")
            .with_context(|| format!("create marker file {marker_path:?}"))?;

        Ok(image_tag)
    }
}

#[bon]
impl Container {
    /// Start the container and return a reference to it.
    #[builder(start_fn = new, finish_fn = start)]
    pub async fn build(
        /// Commands to run via exec after the container is started.
        ///
        /// These are executed by calling `command.run_docker()` after the container
        /// is running. Use this for setup tasks, initialization scripts, or any
        /// commands that need to run in an already-running container.
        ///
        /// This is different from `entrypoint`: commands run after the container's
        /// main process has started, while entrypoint replaces the main process.
        #[builder(field)]
        commands: Vec<Command>,

        /// Volume binds to mount in the container.
        /// Each tuple represents (host_path, container_path).
        #[builder(field)]
        volume_binds: Vec<(PathBuf, PathBuf)>,

        /// Environment variables to set in the container.
        /// Each tuple represents (key, value).
        #[builder(field)]
        env_vars: Vec<(String, String)>,

        /// The repository to use, in OCI format.
        #[builder(into)]
        repo: String,

        /// The tag to use.
        #[builder(into)]
        tag: String,

        /// Optional Docker network ID to attach the container to.
        #[builder(into)]
        network: Option<String>,

        /// Optional container name for DNS resolution within Docker networks.
        #[builder(into)]
        container_name: Option<String>,

        /// Override the container's CMD (the command that runs as the main process).
        ///
        /// This replaces the CMD specified in the Docker image. The container will
        /// run this command as its main process and exit when it completes.
        ///
        /// This is different from `commands`: entrypoint is the main process that
        /// determines the container's lifecycle, while commands are executed via
        /// exec in an already-running container.
        ///
        /// Example: To run migrations in a courier container:
        /// ```ignore
        /// .entrypoint(["migrate", "--database-url", "postgres://..."])
        /// ```
        #[builder(default, with = |i: impl IntoIterator<Item = impl Into<String>>| i.into_iter().map(Into::into).collect())]
        entrypoint: Vec<String>,
    ) -> Result<Container> {
        let docker = Docker::connect_with_defaults().context("connect to docker daemon")?;
        let reference = format!("{repo}:{tag}");

        // Only pull images from registries (e.g., docker.io/library/rust).
        // Skip pulling for locally-built images (e.g., hurry-courier:test).
        if repo.contains('/') {
            let image = CreateImageOptionsBuilder::new()
                .from_image(&reference)
                .build();
            docker
                .create_image(Some(image), None, None)
                .inspect_ok(|msg| println!("[IMAGE] {msg:?}"))
                .try_collect::<Vec<_>>()
                .await?;
        }

        let container_opts = if let Some(name) = container_name {
            CreateContainerOptionsBuilder::default()
                .name(&name)
                .build()
        } else {
            CreateContainerOptionsBuilder::default().build()
        };

        let host_config = if volume_binds.is_empty() {
            None
        } else {
            let bind_strings = volume_binds
                .into_iter()
                .map(|(host_path, container_path)| {
                    let host_path = host_path
                        .to_str()
                        .ok_or_eyre("invalid string")
                        .with_context(|| format!("convert host path to string: {host_path:?}"))?;
                    let container_path = container_path
                        .to_str()
                        .ok_or_eyre("invalid string")
                        .with_context(|| {
                            format!("convert container path to string: {container_path:?}")
                        })?;
                    Ok(format!("{host_path}:{container_path}:z,rw"))
                })
                .collect::<Result<Vec<_>>>()
                .context("convert volume binds")?;
            Some(HostConfig {
                binds: Some(bind_strings),
                ..Default::default()
            })
        };

        let env = if env_vars.is_empty() {
            None
        } else {
            Some(
                env_vars
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>(),
            )
        };

        let networking_config = network.map(|network_id| {
            let mut endpoints = HashMap::new();
            endpoints.insert(network_id, EndpointSettings::default());
            NetworkingConfig {
                endpoints_config: Some(endpoints),
            }
        });

        let cmd = if entrypoint.is_empty() {
            None
        } else {
            Some(entrypoint)
        };

        let container_body = ContainerCreateBody {
            image: Some(reference),
            tty: Some(true),
            host_config,
            env,
            networking_config,
            cmd,
            ..Default::default()
        };
        let id = docker
            .create_container(Some(container_opts), container_body)
            .await
            .context("create container")?
            .id;

        let start_opts = StartContainerOptionsBuilder::default().build();
        docker
            .start_container(&id, Some(start_opts))
            .await
            .context("start container")?;
        let container = Container {
            inner: Arc::new(ContainerRef { id, docker }),
        };

        for command in commands {
            command
                .run_docker(&container)
                .await
                .context("run command in docker context")?;
        }

        Ok(container)
    }

    /// Start a Debian container capable of running Rust builds.
    ///
    /// Builds this container with the `latest` tag:
    /// https://hub.docker.com/_/rust
    #[builder(finish_fn = start)]
    pub async fn debian_rust(
        #[builder(field)] commands: Vec<Command>,
        #[builder(field)] volume_binds: Vec<(PathBuf, PathBuf)>,
        #[builder(field)] env_vars: Vec<(String, String)>,
        #[builder(into)] network: Option<String>,
        #[builder(into)] container_name: Option<String>,
    ) -> Result<Container> {
        Container::new()
            .repo("docker.io/library/rust")
            .tag("latest")
            .commands(commands)
            .volume_binds(volume_binds)
            .env_vars(env_vars)
            .maybe_network(network)
            .maybe_container_name(container_name)
            .start()
            .await
    }
}

impl<S: container_build_builder::State> ContainerBuildBuilder<S> {
    /// Add commands to run when the container is started.
    pub fn commands(mut self, commands: impl IntoIterator<Item = impl Into<Command>>) -> Self {
        self.commands.extend(commands.into_iter().map(Into::into));
        self
    }

    /// Add a command to run when the container is started.
    pub fn command(mut self, command: impl Into<Command>) -> Self {
        self.commands.push(command.into());
        self
    }

    /// Add volume binds to mount in the container.
    /// Each tuple represents (host_path, container_path).
    pub fn volume_binds(
        mut self,
        binds: impl IntoIterator<Item = (impl Into<PathBuf>, impl Into<PathBuf>)>,
    ) -> Self {
        self.volume_binds
            .extend(binds.into_iter().map(|(h, c)| (h.into(), c.into())));
        self
    }

    /// Add a single volume bind to mount in the container.
    pub fn volume_bind(
        mut self,
        host_path: impl Into<PathBuf>,
        container_path: impl Into<PathBuf>,
    ) -> Self {
        self.volume_binds
            .push((host_path.into(), container_path.into()));
        self
    }

    /// Add environment variables to set in the container.
    /// Each tuple represents (key, value).
    pub fn env_vars(
        mut self,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.env_vars
            .extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Add a single environment variable to set in the container.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), value.into()));
        self
    }
}

impl<S: container_debian_rust_builder::State> ContainerDebianRustBuilder<S> {
    /// Add commands to run when the container is started.
    pub fn commands(mut self, commands: impl IntoIterator<Item = impl Into<Command>>) -> Self {
        self.commands.extend(commands.into_iter().map(Into::into));
        self
    }

    /// Add a command to run when the container is started.
    pub fn command(mut self, command: impl Into<Command>) -> Self {
        self.commands.push(command.into());
        self
    }

    /// Add volume binds to mount in the container.
    /// Each tuple represents (host_path, container_path).
    pub fn volume_binds(
        mut self,
        binds: impl IntoIterator<Item = (impl Into<PathBuf>, impl Into<PathBuf>)>,
    ) -> Self {
        self.volume_binds
            .extend(binds.into_iter().map(|(h, c)| (h.into(), c.into())));
        self
    }

    /// Add a single volume bind to mount in the container.
    /// Takes (host_path, container_path) tuple.
    pub fn volume_bind(
        mut self,
        host_path: impl Into<PathBuf>,
        container_path: impl Into<PathBuf>,
    ) -> Self {
        self.volume_binds
            .push((host_path.into(), container_path.into()));
        self
    }

    /// Add environment variables to set in the container.
    /// Each tuple represents (key, value).
    pub fn env_vars(
        mut self,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.env_vars
            .extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Add a single environment variable to set in the container.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), value.into()));
        self
    }
}

/// Internally references a running Docker container; when this is dropped the
/// container is removed from Docker.
#[derive(Debug)]
struct ContainerRef {
    docker: Docker,
    id: String,
}

impl Drop for ContainerRef {
    fn drop(&mut self) {
        let id = self.id.clone();
        let docker = self.docker.clone();

        // This is not a place of honor. No highly esteemed deed is commemorated
        // here. Nothing valued is here. What is here was dangerous and
        // repulsive to us.
        //
        // The difficulty here is:
        // - We want to clean up containers when they're no longer needed.
        // - Assertion failures or errors won't run manual cleanup code.
        // - `bollard` is async, while `Drop` is not.
        //
        // We can't just spawn the cleanup task; the test just exits without
        // actually cleaning anything up unless we actually block the drop
        // function.
        //
        // This spawns a new thread + runtime per drop, which makes it only
        // suitable for expensive resources like Docker containers (where the
        // cost of spawning a new thread is nothing compared to the cost of the
        // network calls to create and tear down containers). For lighter
        // cleanup tasks, or if you just want to make something less gross,
        // consider spawning a long-lived cleanup thread with a dedicated async
        // runtime and sending cleanup tasks to it through a channel (which
        // probably means also sending a channel into the cleanup task so that
        // drop can wait for the cleanup to actually complete).
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("create runtime for cleanup");
            rt.block_on(async move {
                let options = RemoveContainerOptionsBuilder::new()
                    .force(true)
                    .v(true)
                    .build();

                if let Err(err) = docker.remove_container(&id, Some(options)).await {
                    eprintln!("[WARN] Unable to remove container {id}: {err:?}");
                }
            });
        });

        // Wait for cleanup to complete. This blocks the drop function, which
        // correctly prevents the test from exiting before it cleans up.
        if let Err(panic) = handle.join() {
            std::panic::resume_unwind(panic);
        }
    }
}
