use clap::Args;
use color_eyre::{Result, eyre::Context as _};
use hurry::{daemon::DaemonPaths, fs, path::AbsFilePath};
use std::io::Write as _;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt as _, BufReader};
use tracing::instrument;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Follow the log file like `tail -f`
    #[arg(short, long)]
    follow: bool,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let paths = DaemonPaths::initialize().await?;

    let Some(daemon_context) = paths.read_context().await? else {
        eprintln!("Daemon not running (no context file found)");
        return Ok(());
    };

    let log_path = &daemon_context.log_file_path;

    if !log_path.exists().await {
        eprintln!("Log file not found: {log_path}");
        return Ok(());
    }

    if options.follow {
        follow_log(log_path).await
    } else {
        print_log(log_path).await
    }
}

async fn print_log(log_path: &AbsFilePath) -> Result<()> {
    let content = fs::read_buffered_utf8(log_path)
        .await
        .context("read log file")?
        .unwrap_or_default();

    print!("{content}");
    Ok(())
}

async fn follow_log(log_path: &AbsFilePath) -> Result<()> {
    let content = fs::read_buffered_utf8(log_path)
        .await
        .context("read existing log content")?
        .unwrap_or_default();

    print!("{content}");
    std::io::stdout().flush().context("flush stdout")?;

    let mut file = fs::open_file(log_path)
        .await
        .context("open log file for following")?;

    file.seek(std::io::SeekFrom::End(0))
        .await
        .context("seek to end of log file")?;

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await.context("read line")?;

        if n == 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            continue;
        }

        print!("{line}");
        std::io::stdout().flush().context("flush stdout")?;
    }
}
