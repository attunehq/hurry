use clap::Args;
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _},
};
use hurry::{daemon::DaemonPaths, fs};
use tracing::instrument;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Print just a specific field value (e.g., "log_file_path", "pid", "url")
    field: Option<String>,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;

    if !paths.context_path.exists().await {
        eprintln!("Daemon not running (no context file found)");
        return Ok(());
    }

    let context = fs::read_buffered_utf8(&paths.context_path)
        .await
        .context("read daemon context file")?
        .ok_or_eyre("no daemon context file")?;

    if let Some(field) = options.field {
        let daemon_context = serde_json::from_str::<hurry::daemon::DaemonReadyMessage>(&context)
            .context("parse daemon context")?;

        let value = match field.as_str() {
            "pid" => daemon_context.pid.to_string(),
            "url" => daemon_context.url,
            "log_file_path" => daemon_context.log_file_path.to_string(),
            _ => {
                eprintln!("Unknown field: {field}");
                eprintln!("Valid fields: pid, url, log_file_path");
                return Ok(());
            }
        };

        println!("{value}");
    } else {
        print!("{context}");
    }

    Ok(())
}
