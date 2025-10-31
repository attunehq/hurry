use clap::Args;
use color_eyre::Result;
use hurry::daemon::DaemonPaths;
use tracing::instrument;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Print just a specific field value (e.g., "log_file_path", "pid", "url")
    field: Option<String>,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;

    let Some(daemon_context) = paths.read_context().await? else {
        eprintln!("Daemon not running (no context file found)");
        return Ok(());
    };

    if let Some(field) = options.field {
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
        let context = serde_json::to_string_pretty(&daemon_context)?;
        print!("{context}");
    }

    Ok(())
}
