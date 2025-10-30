use clap::Args;
use color_eyre::Result;
use hurry::daemon::DaemonPaths;
use tracing::instrument;

#[derive(Clone, Args, Debug)]
pub struct Options {}

#[instrument]
pub async fn exec(_options: Options) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;

    if paths.daemon_running().await? {
        println!("running");
    } else {
        println!("stopped");
    }

    Ok(())
}
