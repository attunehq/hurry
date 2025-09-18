use std::sync::Arc;

use bollard::{
    Docker,
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
        StartContainerOptionsBuilder,
    },
    secret::ContainerCreateBody,
};
use bon::bon;
use color_eyre::{Result, eyre::Context};
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
}

#[bon]
impl Container {
    /// Start the container and return a reference to it.
    #[builder(start_fn = new, finish_fn = start)]
    pub async fn build(
        /// Commands to run when the container is started.
        #[builder(field)]
        commands: Vec<Command>,

        /// The repository to use, in OCI format.
        #[builder(into)]
        repo: String,

        /// The tag to use.
        #[builder(into)]
        tag: String,
    ) -> Result<Container> {
        let docker = Docker::connect_with_defaults().context("connect to docker daemon")?;
        let reference = format!("{repo}:{tag}");

        let image = CreateImageOptionsBuilder::new()
            .from_image(&reference)
            .build();
        docker
            .create_image(Some(image), None, None)
            .inspect_ok(|msg| println!("[IMAGE] {msg:?}"))
            .try_collect::<Vec<_>>()
            .await?;

        let container_opts = CreateContainerOptionsBuilder::default().build();
        let container_body = ContainerCreateBody {
            image: Some(reference),
            tty: Some(true),
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
    pub async fn debian_rust(#[builder(field)] commands: Vec<Command>) -> Result<Container> {
        Container::new()
            .repo("docker.io/library/rust")
            .tag("latest")
            .commands(commands)
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
}

/// Internally references a running Docker container.
///
/// Attempts to remove the container from Docker when dropped, although since
/// there is no such thing as async drop yet this is best effort and will likely
/// lead to many orphan containers.
#[derive(Debug)]
struct ContainerRef {
    docker: Docker,
    id: String,
}

impl Drop for ContainerRef {
    fn drop(&mut self) {
        let id = self.id.clone();
        let docker = self.docker.clone();
        tokio::task::spawn(async move {
            let options = RemoveContainerOptionsBuilder::new()
                .force(true)
                .v(true)
                .build();

            if let Err(err) = docker.remove_container(&id, Some(options)).await {
                eprintln!("[WARN] Unable to remove container {id}: {err:?}");
            }
        });
    }
}
