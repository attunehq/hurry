use clap::Args;
use color_eyre::Result;
use hurry::daemon::DaemonPaths;
use tracing::instrument;

#[derive(Clone, Args, Debug)]
pub struct Options {}

#[instrument]
pub async fn exec(_options: Options) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;

    match paths.daemon_running().await? {
        Some(_) => println!("running"),
        None => println!("stopped"),
    }

    Ok(())
}
