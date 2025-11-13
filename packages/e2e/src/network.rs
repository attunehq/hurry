use std::sync::Arc;

use bollard::{Docker, models::NetworkCreateRequest};
use color_eyre::{Result, eyre::Context};
use uuid::Uuid;

/// References a Docker network.
///
/// This reference uses interior mutability to track internally track references
/// to the network. After the final reference is dropped, attempts to remove
/// the network from the docker daemon.
///
/// Attempts to remove the network from Docker when dropped, although since
/// there is no such thing as async drop yet this is best effort and will likely
/// lead to many orphan networks.
#[derive(Clone, Debug)]
pub struct Network {
    inner: Arc<NetworkRef>,
}

impl Network {
    /// Create a new Docker network with a unique name.
    ///
    /// The network name will be in the format `e2e-test-{uuid}`.
    pub async fn create() -> Result<Self> {
        let docker = Docker::connect_with_defaults().context("connect to docker daemon")?;
        let name = format!("e2e-test-{}", Uuid::new_v4());

        let config = NetworkCreateRequest {
            name: name.clone(),
            ..Default::default()
        };

        let response = docker
            .create_network(config)
            .await
            .context("create docker network")?;

        let id = response.id;

        Ok(Network {
            inner: Arc::new(NetworkRef { docker, id, name }),
        })
    }

    /// Reference to the Docker client.
    pub fn docker(&self) -> &Docker {
        &self.inner.docker
    }

    /// The ID of the network in the docker context.
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// The name of the network.
    pub fn name(&self) -> &str {
        &self.inner.name
    }
}

/// Internally references a Docker network; when this is dropped the
/// network is removed from Docker.
#[derive(Debug)]
struct NetworkRef {
    docker: Docker,
    id: String,
    name: String,
}

impl Drop for NetworkRef {
    fn drop(&mut self) {
        let name = self.name.clone();
        let docker = self.docker.clone();

        // This is not a place of honor. No highly esteemed deed is commemorated
        // here. Nothing valued is here. What is here was dangerous and
        // repulsive to us.
        //
        // The difficulty here is:
        // - We want to clean up networks when they're no longer needed.
        // - Assertion failures or errors won't run manual cleanup code.
        // - `bollard` is async, while `Drop` is not.
        //
        // We can't just spawn the cleanup task; the test just exits without
        // actually cleaning anything up unless we actually block the drop
        // function.
        //
        // This spawns a new thread + runtime per drop, which makes it only
        // suitable for expensive resources like Docker networks (where the
        // cost of spawning a new thread is nothing compared to the cost of the
        // network calls to create and tear down networks). For lighter
        // cleanup tasks, or if you just want to make something less gross,
        // consider spawning a long-lived cleanup thread with a dedicated async
        // runtime and sending cleanup tasks to it through a channel (which
        // probably means also sending a channel into the cleanup task so that
        // drop can wait for the cleanup to actually complete).
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("create runtime for cleanup");
            rt.block_on(async move {
                if let Err(err) = docker.remove_network(&name).await {
                    eprintln!("[WARN] Unable to remove network {name}: {err:?}");
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
