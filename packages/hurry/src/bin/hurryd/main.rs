// 1. Try to open a Unix socket at a known location in the Hurry cache. Die if this file already exists and the server at the file responds to pings. (This is functionally a pid-file.)
// 2. Start an HTTP control plane server over the socket and listen for HTTP requests.
//
// Control plane API:
// - GET /status: Return current uploads in progress.
// - POST /upload: Given a payload of local file paths with associated packages, upload the packages asynchronously to CAS.
//
// State for things like uploads is stored in-memory, and lost on crash.
//
// In the end-to-end flow, when Hurry CLI is doing a restore:
// 1. Copy the files into the `hurryd` cache directory. (This will be fast on CoW filesystems.)
// 2. Make a control plane request to upload these files to CAS.
// 3. Control plane stores the upload status of each object and associated packages in a local database. (SQLite, or alternatively Postgres-over-local-socket?)
// 4. This acts as a local pull-through cache of packages and objects so you don't always need to go to the database.

use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use color_eyre::{Result, eyre::bail};
use hurry::{
    mk_rel_file,
    path::{JoinWith, TryJoinWith},
};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tokio::net::UnixListener;
use tower_http::trace::TraceLayer;
use tracing::{debug, info, instrument, warn};
use tracing_error::ErrorLayer;
use tracing_subscriber::{Layer as _, layer::SubscriberExt as _, util::SubscriberInitExt};
use tracing_tree::time::Uptime;

#[instrument]
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize error handling.
    color_eyre::install()?;

    // Set up daemon directory.
    let cache_dir = hurry::fs::user_global_cache_path().await?;
    let pid = std::process::id();
    let socket_path = cache_dir.join(mk_rel_file!("hurryd.sock"));
    let pid_file_path = cache_dir.join(mk_rel_file!("hurryd.pid"));
    let stderr_file_path = cache_dir.try_join_file(format!("hurryd.{}.err", pid))?;

    // Initialize logging.
    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(
            tracing_tree::HierarchicalLayer::default()
                .with_indent_lines(true)
                .with_indent_amount(2)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_verbose_exit(false)
                .with_verbose_entry(false)
                .with_deferred_spans(false)
                .with_bracketed_fields(true)
                .with_span_retrace(true)
                .with_timer(Uptime::default())
                .with_writer(std::fs::File::create(stderr_file_path.as_std_path())?)
                .with_targets(false)
                .with_filter(
                    tracing_subscriber::EnvFilter::builder()
                        .with_env_var("HURRYD_LOG")
                        .from_env_lossy(),
                ),
        )
        .init();

    debug!(
        ?socket_path,
        ?pid_file_path,
        ?stderr_file_path,
        "file paths"
    );

    // If a pid-file exists, read it and check if the process is running. Exit
    // if another instance is running.
    if pid_file_path.exists().await {
        let pid = hurry::fs::must_read_buffered_utf8(&pid_file_path).await?;
        match pid.trim().parse::<u32>() {
            Ok(pid) => {
                let system = System::new_with_specifics(
                    RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
                );
                let process = system.process(Pid::from_u32(pid));
                if process.is_some() {
                    bail!("hurryd is already running at pid {pid}");
                }
            }
            Err(err) => {
                warn!(?err, "could not parse pid-file");
            }
        };
    }

    // Write and lock a pid-file.
    let mut pid_file = fslock::LockFile::open(pid_file_path.as_os_str())?;
    let locked = pid_file.try_lock_with_pid()?;
    if !locked {
        bail!("hurryd is already running");
    }

    // Install a handler that ignores SIGHUP so that terminal exits don't kill
    // the daemon. I can't get anything to work with proper double-fork
    // daemonization so we'll just do this for now.
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGHUP, || {
            warn!("ignoring SIGHUP");
        })?;
    }

    // Open the socket and start the server.
    std::fs::remove_file(&socket_path.as_std_path())?;
    let listener = UnixListener::bind(socket_path.as_std_path())?;
    info!(?socket_path, "server listening");

    let cargo = Router::new()
        .route("/status", get(status))
        .route("/upload", post(upload))
        .with_state(ServerState::default());

    let app = Router::new()
        .nest("/api/v0/cargo", cargo)
        .layer(TraceLayer::new_for_http());
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Debug, Clone)]
struct ServerState {}

impl Default for ServerState {
    fn default() -> Self {
        Self {}
    }
}

async fn status(state: State<ServerState>) {}

struct CargoUploadRequest {}

async fn upload(state: State<ServerState>) {}
