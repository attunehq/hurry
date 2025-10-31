use clap::Args;
use color_eyre::{Result, eyre::bail};
use hurry::daemon::DaemonPaths;
use reqwest::Client;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tokio::time::{Duration, sleep};
use tracing::instrument;

// TODO: We should probably support a `--wait` option or similar that allows
// shutting down the daemon only once all current uploads finish.
#[derive(Clone, Args, Debug)]
pub struct Options {}

#[instrument]
pub async fn exec(_options: Options) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;

    let Some(context) = paths.read_context().await? else {
        println!("Daemon not running");
        return Ok(());
    };

    let url = format!("http://{}/api/v0/shutdown", context.url);
    let client = Client::new();

    match client.post(&url).send().await {
        Ok(_) => {
            println!("Shutdown signal sent, waiting for daemon to exit...");
        }
        Err(err) => {
            bail!("Failed to send shutdown request: {err}");
        }
    }

    let pid = Pid::from_u32(context.pid);
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            bail!("Daemon did not exit within timeout");
        }

        let system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
        );

        if system.process(pid).is_none() {
            println!("Daemon stopped successfully");
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}
