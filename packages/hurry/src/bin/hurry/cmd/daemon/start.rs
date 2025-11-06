use std::time::{Duration, SystemTime};

use axum::{
    Json, Router,
    extract::{FromRef, State},
    routing::post,
};
use clap::Args;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context as _, bail},
};

use derive_more::Debug;
use tokio::signal;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing::{Subscriber, debug, dispatcher, info, instrument, warn};
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;

use crate::{TopLevelFlags, log};
use hurry::{
    daemon::{CargoDaemonState, DaemonContext, DaemonPaths, cargo_router},
    fs,
    path::TryJoinWith,
};

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Base URL for the Courier instance.
    #[arg(
        long = "hurry-courier-url",
        env = "HURRY_COURIER_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{courier_url}")]
    courier_url: Url,
}

/// Clean up daemon log files older than the specified age.
///
/// This function runs asynchronously in the background and does not block
/// daemon startup. It scans the cache directory for log files matching the
/// pattern `hurryd.*.log` and removes those with modification times older
/// than the specified duration.
#[instrument]
async fn cleanup_old_logs(cache_dir: &hurry::path::AbsDirPath, max_age: Duration) {
    debug!("starting log cleanup task");

    let result: Result<()> = async {
        let mut read_dir = fs::read_dir(cache_dir).await?;
        let mut removed_count = 0;
        let mut error_count = 0;

        let cutoff_time = SystemTime::now()
            .checked_sub(max_age)
            .ok_or_else(|| color_eyre::eyre::eyre!("failed to calculate cutoff time"))?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();

            // Check if the file matches the daemon log pattern
            if let Some(filename) = path.file_name().and_then(|n| n.to_str())
                && filename.starts_with("hurryd.")
                && filename.ends_with(".log")
            {
                // Check the file's modification time
                match fs::metadata(&path).await {
                    Ok(Some(metadata)) => {
                        if let Ok(modified) = metadata.modified()
                            && modified < cutoff_time
                        {
                            // Try to remove the old log file
                            match tokio::fs::remove_file(&path).await {
                                Ok(_) => {
                                    debug!(?path, "removed old log file");
                                    removed_count += 1;
                                }
                                Err(err) => {
                                    warn!(?path, ?err, "failed to remove old log file");
                                    error_count += 1;
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        debug!(?path, "log file does not exist (race condition)");
                    }
                    Err(err) => {
                        warn!(?path, ?err, "failed to get metadata for log file");
                        error_count += 1;
                    }
                }
            }
        }

        info!(removed_count, error_count, "log cleanup completed");

        Ok(())
    }
    .await;

    if let Err(err) = result {
        warn!(?err, "log cleanup task failed");
    }
}

#[instrument(skip(cli_logger))]
pub async fn exec(
    top_level_flags: TopLevelFlags,
    cli_logger: impl Subscriber + Send + Sync,
    options: Options,
) -> Result<()> {
    // Set up daemon directory.
    let cache_dir = hurry::fs::user_global_cache_path().await?;
    hurry::fs::create_dir_all(&cache_dir).await?;

    let paths = DaemonPaths::initialize().await?;
    let pid = std::process::id();
    let log_file_path = cache_dir.try_join_file(format!("hurryd.{}.log", pid))?;

    // Redirect logging into file (for daemon mode). We need to redirect the
    // logging firstly so that we can continue to see logs if the invoking
    // terminal exits, but more importantly because the invoking terminal
    // exiting causes the STDOUT and STDERR pipes of this program to close,
    // which means the process crashes with a SIGPIPE if it attempts to write to
    // them.
    let (file_logger, flame_guard) = dispatcher::with_default(&cli_logger.into(), || {
        debug!(?paths, ?log_file_path, "file paths");
        info!(?log_file_path, "logging to file");

        log::make_logger(
            #[allow(
                clippy::disallowed_methods,
                reason = "sync in main thread is OK, dispatcher closure is sync"
            )]
            std::fs::File::create(log_file_path.as_std_path())?,
            top_level_flags.profile,
            top_level_flags.color,
        )
    })?;
    file_logger.init();

    // Spawn a background task to clean up old log files (older than 3 days).
    // This runs asynchronously and does not block daemon startup.
    {
        let cache_dir = cache_dir.clone();
        tokio::spawn(async move {
            cleanup_old_logs(&cache_dir, Duration::from_secs(3 * 24 * 60 * 60)).await;
        });
    }

    // If a pid-file exists, read it and check if the process is running. Exit
    // if another instance is running.
    if paths.daemon_running().await?.is_some() {
        bail!("hurryd is already running");
    }

    // Write and lock a pid-file.
    let mut pid_file = fslock::LockFile::open(paths.pid_file_path.as_os_str())?;
    if !pid_file.try_lock_with_pid()? {
        bail!("hurryd is already running");
    }

    // Install a handler that ignores SIGHUP so that terminal exits don't kill
    // the daemon. I can't get anything to work with proper double-fork
    // daemonization so we'll just do this for now.
    //
    // The intention of registering this hook is to prevent hurry from closing if
    // the parent shell that launched it is closed. In Windows however, processes
    // are not automatically signaled to exit when their parent exits: launching
    // a program inside a CMD or Powershell instance and then closing that session
    // does not make the program close. Given this I don't think there's a need to
    // do anything special here in Windows.
    //
    // TODO: Validate whether the daemon actually works in Windows or if we need
    // additional setup when launching it.
    #[cfg(unix)]
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGHUP, || {
            warn!("ignoring SIGHUP");
        })?;
    }

    // Bind to port 0 to get a random ephemeral port from the OS. Since this binds
    // an ephemeral port, this does not conflict with typical userspace ports (3000,
    // 8000, 8080, etc) or service ports.
    //
    // Linux ip(7): "An ephemeral port is allocated to a socket in the following
    // circumstances: [...] the port number in a socket address is specified as 0
    // when calling bind(2)". I can't find macOS developer docs that explicitly
    // document this, but from observed behavior it appears to act the same;
    // it's also relatively rare for core functionality like this to diverge
    // between Linux and macOS.
    //
    // Windows bind(): "For TCP/IP, if the port is specified as zero, the service
    // provider assigns a unique port to the application from the dynamic client
    // port range. On Windows Vista and later, the dynamic client port range is a
    // value between 49152 and 65535."
    //
    // References:
    // - https://man7.org/linux/man-pages/man7/ip.7.html (see ip_local_port_range)
    // - https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-bind
    // - https://stackoverflow.com/questions/5895751 (portability/macOS discussion)
    let listener = tokio::net::TcpListener::bind("localhost:0")
        .await
        .context("open local server")?;
    let addr = listener
        .local_addr()
        .context("read listen address for socket")?;
    info!(?addr, "server listening");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let state = ServerState {
        cargo: CargoDaemonState::default(),
        shutdown_tx,
    };

    let app = Router::new()
        .nest(
            "/api/v0/cargo",
            cargo_router().with_state(state.cargo.clone()),
        )
        .route("/api/v0/shutdown", post(shutdown))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    // Write context file for daemon clients.
    let message = DaemonContext {
        pid,
        url: format!("{addr}"),
        log_file_path,
    };
    let encoded = serde_json::to_string(&message)
        .context("encode ready message")
        .with_section(|| format!("{message:?}").header("Message:"))?;
    fs::write(&paths.context_path, &encoded)
        .await
        .with_context(|| format!("write daemon context to {:?}", paths.context_path))?;

    // We don't immediately handle the error with `?` here so that we can perform
    // the cleanup operations regardless of whether an error occurred.
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_rx))
        .await
        .context("start server");

    info!(?paths, "exiting; cleaning up context files");
    if let Err(err) = fs::remove_file(&paths.pid_file_path).await {
        warn!(?err, path = ?paths.pid_file_path, "failed to remove pid file");
    }
    if let Err(err) = fs::remove_file(&paths.context_path).await {
        warn!(?err, path = ?paths.context_path, "failed to remove context file");
    }
    info!("context files cleaned up");

    // TODO: Unsure if we need to keep this, the guard _should_ flush on drop.
    if let Some(flame_guard) = flame_guard {
        flame_guard.flush().context("flush flame_guard")?;
    }

    served
}

/// Wait for a shutdown signal from either OS signals (SIGINT/SIGTERM) or the
/// explicit shutdown channel.
async fn shutdown_signal(mut shutdown_rx: watch::Receiver<bool>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let explicit_shutdown = async {
        let _ = shutdown_rx.changed().await;
    };

    tokio::select! {
        _ = ctrl_c => {
            info!("received SIGINT (Ctrl+C), starting graceful shutdown");
        },
        _ = terminate => {
            info!("received SIGTERM, starting graceful shutdown");
        },
        _ = explicit_shutdown => {
            info!("received explicit shutdown request, starting graceful shutdown");
        },
    }
}

#[derive(Debug, Clone, FromRef)]
struct ServerState {
    cargo: CargoDaemonState,
    shutdown_tx: watch::Sender<bool>,
}
#[instrument]
async fn shutdown(State(state): State<ServerState>) -> Json<serde_json::Value> {
    info!("shutdown request received");

    let _ = state.shutdown_tx.send(true);

    Json(serde_json::json!({ "ok": true }))
}
