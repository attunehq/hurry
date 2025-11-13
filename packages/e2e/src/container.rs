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
    /// # Arguments
    /// - `image_tag`: The tag to apply to the built image (e.g., "hurry-courier:test")
    /// - `dockerfile`: Path to the Dockerfile relative to workspace root (e.g., "docker/courier/Dockerfile")
    /// - `context`: Build context directory relative to workspace root (typically ".")
    ///
    /// # Example
    /// ```ignore
    /// Container::ensure_built(
    ///     "hurry-courier:test",
    ///     "docker/courier/Dockerfile",
    ///     ".",
    /// ).await?;
    /// ```
    pub async fn ensure_built(
        image_tag: impl AsRef<str>,
        dockerfile: impl AsRef<Path>,
        context: impl AsRef<Path>,
    ) -> Result<()> {
        let image_tag = image_tag.as_ref();
        let dockerfile = dockerfile.as_ref();
        let context = context.as_ref();

        let workspace_root = workspace_root::get_workspace_root();
        let target_dir = workspace_root.join("target");

        let sanitized_tag = image_tag.replace(':', "_").replace('/', "_");
        let marker_path = target_dir.join(format!(".{sanitized_tag}.built"));
        let lock_path = target_dir.join(format!(".{sanitized_tag}.lock"));

        // Fast path: check if marker file exists
        if marker_path.exists() {
            return Ok(());
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
            return Ok(());
        }

        // Build the image
        eprintln!("[BUILD] Building Docker image {image_tag}...");

        let dockerfile_path = workspace_root.join(dockerfile);
        let context_path = workspace_root.join(context);

        let status = std::process::Command::new("docker")
            .args([
                "build",
                "-t",
                image_tag,
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

        Ok(())
    }
}

#[bon]
impl Container {
    /// Start the container and return a reference to it.
    #[builder(start_fn = new, finish_fn = start)]
    pub async fn build(
        /// Commands to run when the container is started.
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

        let container_body = ContainerCreateBody {
            image: Some(reference),
            tty: Some(true),
            host_config,
            env,
            networking_config,
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
