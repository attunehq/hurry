use std::{collections::HashSet, io::Write, sync::Arc, time::UNIX_EPOCH};

use axum::{Json, Router, extract::State, routing::post};
use clap::Args;
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context as _, OptionExt as _, bail},
};
use dashmap::DashMap;
use derive_more::Debug;
use futures::{TryStreamExt as _, stream};
use itertools::Itertools as _;
use tap::Pipe as _;
use tokio::signal;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing::{Subscriber, debug, dispatcher, error, info, instrument, trace, warn};
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;
use uuid::Uuid;

use crate::{TopLevelFlags, log};
use clients::{
    Courier,
    courier::v1::{
        Key,
        cache::{ArtifactFile, CargoSaveRequest},
    },
};
use hurry::{
    cargo::{
        BuildScriptOutput, BuiltArtifact, DepInfo, LibraryUnitHash, QualifiedPath, RootOutput,
        Workspace,
    },
    cas::CourierCas,
    daemon::{
        CargoUploadRequest, CargoUploadResponse, CargoUploadStatus, CargoUploadStatusRequest,
        CargoUploadStatusResponse, DaemonPaths, DaemonReadyMessage,
    },
    fs,
    path::{AbsFilePath, TryJoinWith},
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

    // If a pid-file exists, read it and check if the process is running. Exit
    // if another instance is running.
    if paths.daemon_running().await? {
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

    // Open the socket and start the server.
    match fs::remove_file(&paths.context_path).await {
        Ok(_) => {}
        Err(err) => {
            let err = err.downcast::<std::io::Error>()?;
            if err.kind() != std::io::ErrorKind::NotFound {
                error!(?err, "could not remove socket file");
                bail!("could not remove socket file");
            }
        }
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
        uploads: DashMap::new().into(),
        shutdown_tx,
    };

    let cargo = Router::new()
        .route("/upload", post(upload))
        .route("/status", post(status))
        .with_state(state.clone());

    let app = Router::new()
        .nest("/api/v0/cargo", cargo)
        .route("/api/v0/shutdown", post(shutdown))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    // Print ready message to STDOUT for parent processes. This uses `println!`
    // instead of the tracing macros because it emits a special sentinel value
    // on STDOUT.
    let message = DaemonReadyMessage {
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
    println!("{encoded}");

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

#[derive(Debug, Clone)]
struct ServerState {
    uploads: Arc<DashMap<Uuid, CargoUploadStatus>>,
    shutdown_tx: watch::Sender<bool>,
}

#[instrument]
async fn status(
    State(state): State<ServerState>,
    Json(req): Json<CargoUploadStatusRequest>,
) -> Json<CargoUploadStatusResponse> {
    let status = state
        .uploads
        .get(&req.request_id)
        .map(|r| r.value().to_owned());
    Json(CargoUploadStatusResponse { status })
}

#[instrument]
async fn shutdown(State(state): State<ServerState>) -> Json<serde_json::Value> {
    info!("shutdown request received");

    let _ = state.shutdown_tx.send(true);

    Json(serde_json::json!({ "ok": true }))
}

#[instrument]
async fn upload(
    State(state): State<ServerState>,
    Json(req): Json<CargoUploadRequest>,
) -> Json<CargoUploadResponse> {
    let request_id = req.request_id;
    state.uploads.insert(
        request_id,
        CargoUploadStatus::InProgress {
            uploaded_artifacts: 0,
            total_artifacts: req.artifact_plan.artifacts.len() as u64,
            uploaded_files: 0,
            uploaded_bytes: 0,
        },
    );
    tokio::spawn(async move {
        let upload = async {
            trace!(artifact_plan = ?req.artifact_plan, "artifact plan");
            let courier = Courier::new(req.courier_url)?;
            let cas = CourierCas::new(courier.clone());
            let restored_artifacts = HashSet::<_>::from_iter(req.skip_artifacts);
            let restored_objects = HashSet::from_iter(req.skip_objects);

            let mut uploaded_artifacts = 0;
            let mut uploaded_files = 0;
            let mut uploaded_bytes = 0;
            let mut total_artifacts = req.artifact_plan.artifacts.len() as u64;

            for artifact_key in req.artifact_plan.artifacts {
                let artifact = BuiltArtifact::from_key(&req.ws, artifact_key.clone()).await?;
                debug!(?artifact, "caching artifact");

                if restored_artifacts.contains(&artifact_key) {
                    trace!(
                        ?artifact_key,
                        "skipping backup: artifact was restored from cache"
                    );
                    total_artifacts -= 1;
                    continue;
                }

                let lib_files = collect_library_files(&req.ws, &artifact).await?;
                let build_script_files = collect_build_script_files(&req.ws, &artifact).await?;
                let files_to_save = lib_files.into_iter().chain(build_script_files).collect();
                let (library_unit_files, artifact_files, bulk_entries) =
                    process_files_for_upload(&req.ws, files_to_save, &restored_objects).await?;

                let (bytes, files) = upload_files_bulk(&cas, bulk_entries).await?;
                uploaded_bytes += bytes;
                uploaded_files += files;

                let content_hash = calculate_content_hash(library_unit_files)?;
                debug!(?content_hash, "calculated content hash");

                let request = build_save_request(
                    &artifact,
                    &req.artifact_plan.target,
                    content_hash,
                    artifact_files,
                );

                courier.cargo_cache_save(request).await?;
                uploaded_artifacts += 1;
                state.uploads.insert(
                    request_id,
                    CargoUploadStatus::InProgress {
                        uploaded_artifacts,
                        total_artifacts,
                        uploaded_files,
                        uploaded_bytes,
                    },
                );
            }

            Result::<_>::Ok(())
        }
        .await;
        match upload {
            Ok(()) => {
                info!(?request_id, "upload completed successfully");
                state
                    .uploads
                    .insert(request_id, CargoUploadStatus::Complete);
            }
            Err(err) => {
                error!(?err, ?request_id, "upload failed");
                state
                    .uploads
                    .insert(request_id, CargoUploadStatus::Complete);
            }
        }
    });
    Json(CargoUploadResponse { ok: true })
}

#[instrument(skip(content))]
async fn rewrite(ws: &Workspace, path: &AbsFilePath, content: &[u8]) -> Result<Vec<u8>> {
    // Determine what kind of file this is based on path structure.
    let components = path.component_strs_lossy().collect::<Vec<_>>();

    // Look at the last few components to determine file type.
    // We use .rev() to start from the filename and work backwards.
    let file_type = components
        .iter()
        .rev()
        .tuple_windows::<(_, _, _)>()
        .find_map(|(name, parent, gparent)| {
            let ext = name.as_ref().rsplit_once('.').map(|(_, ext)| ext);
            match (gparent.as_ref(), parent.as_ref(), name.as_ref(), ext) {
                ("build", _, "output", _) => Some("build-script-output"),
                ("build", _, "root-output", _) => Some("root-output"),
                (_, _, _, Some("d")) => Some("dep-info"),
                _ => None,
            }
        });

    match file_type {
        Some("root-output") => {
            trace!(?path, "rewriting root-output file");
            let parsed = RootOutput::from_file(ws, path).await?;
            serde_json::to_vec(&parsed).context("serialize RootOutput")
        }
        Some("build-script-output") => {
            trace!(?path, "rewriting build-script-output file");
            let parsed = BuildScriptOutput::from_file(ws, path).await?;
            serde_json::to_vec(&parsed).context("serialize BuildScriptOutput")
        }
        Some("dep-info") => {
            trace!(?path, "rewriting dep-info file");
            let parsed = DepInfo::from_file(ws, path).await?;
            serde_json::to_vec(&parsed).context("serialize DepInfo")
        }
        None => {
            // No rewriting needed, store as-is.
            Ok(content.to_vec())
        }
        Some(unknown) => {
            bail!("unknown file type for rewriting: {unknown}")
        }
    }
}

/// Collect library files and their fingerprints for an artifact.
async fn collect_library_files(
    ws: &Workspace,
    artifact: &BuiltArtifact,
) -> Result<Vec<AbsFilePath>> {
    let lib_fingerprint_dir = ws.profile_dir.try_join_dirs(&[
        String::from(".fingerprint"),
        format!(
            "{}-{}",
            artifact.package_name, artifact.library_crate_compilation_unit_hash
        ),
    ])?;
    let lib_fingerprint_files = fs::walk_files(&lib_fingerprint_dir)
        .try_collect::<Vec<_>>()
        .await?;
    artifact
        .lib_files
        .iter()
        .cloned()
        .chain(lib_fingerprint_files)
        .collect::<Vec<_>>()
        .pipe(Ok)
}

/// Collect build script files and their fingerprints for an artifact.
async fn collect_build_script_files(
    ws: &Workspace,
    artifact: &BuiltArtifact,
) -> Result<Vec<AbsFilePath>> {
    let Some(ref build_script_files) = artifact.build_script_files else {
        return Ok(vec![]);
    };

    let compiled_files = fs::walk_files(&build_script_files.compiled_dir)
        .try_collect::<Vec<_>>()
        .await?;
    let compiled_fingerprint_dir = ws.profile_dir.try_join_dirs(&[
        String::from(".fingerprint"),
        format!(
            "{}-{}",
            artifact.package_name,
            artifact
                .build_script_compilation_unit_hash
                .as_ref()
                .expect("build script files have compilation unit hash")
        ),
    ])?;
    let compiled_fingerprint_files = fs::walk_files(&compiled_fingerprint_dir)
        .try_collect::<Vec<_>>()
        .await?;
    let output_files = fs::walk_files(&build_script_files.output_dir)
        .try_collect::<Vec<_>>()
        .await?;
    let output_fingerprint_dir = ws.profile_dir.try_join_dirs(&[
        String::from(".fingerprint"),
        format!(
            "{}-{}",
            artifact.package_name,
            artifact
                .build_script_execution_unit_hash
                .as_ref()
                .expect("build script files have execution unit hash")
        ),
    ])?;
    let output_fingerprint_files = fs::walk_files(&output_fingerprint_dir)
        .try_collect::<Vec<_>>()
        .await?;

    compiled_files
        .into_iter()
        .chain(compiled_fingerprint_files)
        .chain(output_files)
        .chain(output_fingerprint_files)
        .collect::<Vec<_>>()
        .pipe(Ok)
}

/// Process files for upload: read, rewrite, calculate keys, and prepare
/// metadata.
async fn process_files_for_upload(
    ws: &Workspace,
    files: Vec<AbsFilePath>,
    restored_objects: &HashSet<Key>,
) -> Result<(
    Vec<(QualifiedPath, Key)>,
    Vec<ArtifactFile>,
    Vec<(Key, Vec<u8>, AbsFilePath)>,
)> {
    let mut library_unit_files = vec![];
    let mut artifact_files = vec![];
    let mut bulk_entries = vec![];

    for path in files {
        let Some(content) = fs::read_buffered(&path).await? else {
            warn!("failed to read file: {}", path);
            continue;
        };

        let content = rewrite(ws, &path, &content).await?;
        let key = Key::from_buffer(&content);

        let metadata = fs::Metadata::from_file(&path)
            .await?
            .ok_or_eyre("could not stat file metadata")?;
        let mtime_nanos = metadata.mtime.duration_since(UNIX_EPOCH)?.as_nanos();
        let qualified = QualifiedPath::parse(ws, path.as_std_path()).await?;

        library_unit_files.push((qualified.clone(), key.clone()));
        artifact_files.push(
            ArtifactFile::builder()
                .object_key(key.clone())
                .path(serde_json::to_string(&qualified)?)
                .mtime_nanos(mtime_nanos)
                .executable(metadata.executable)
                .build(),
        );

        if restored_objects.contains(&key) {
            trace!(?path, ?key, "skipping backup: file was restored from cache");
        } else {
            bulk_entries.push((key, content, path));
        }
    }

    Ok((library_unit_files, artifact_files, bulk_entries))
}

/// Upload files in bulk and return the number of bytes transferred.
async fn upload_files_bulk(
    cas: &CourierCas,
    bulk_entries: Vec<(Key, Vec<u8>, AbsFilePath)>,
) -> Result<(u64, u64)> {
    if bulk_entries.is_empty() {
        return Ok((0, 0));
    }

    debug!(count = bulk_entries.len(), "uploading files");

    let result = bulk_entries
        .iter()
        .map(|(key, content, _)| (key.clone(), content.clone()))
        .collect::<Vec<_>>()
        .pipe(stream::iter)
        .pipe(|stream| cas.store_bulk(stream))
        .await
        .context("upload batch")?;

    let mut uploaded_bytes = 0u64;
    let mut uploaded_files = 0u64;
    for (key, content, path) in &bulk_entries {
        if result.written.contains(key) {
            uploaded_bytes += content.len() as u64;
            uploaded_files += 1;
            debug!(?path, ?key, "uploaded via bulk");
        } else if result.skipped.contains(key) {
            debug!(?path, ?key, "skipped by server (already exists)");
        } else {
            // TODO: Look up the actual error for the key. If a key is not in
            // written, skipped, or errors, then something has gone seriously
            // wrong. To make this more ergonomic, we should probably refactor
            // the errors into a `BTreeMap<Key, String>`.
            warn!(?path, ?key, "failed to upload file in bulk operation");
        }
    }

    for error in &result.errors {
        warn!(
            key = ?error.key,
            error = %error.error,
            "failed to upload file in bulk operation"
        );
    }

    Ok((uploaded_bytes, uploaded_files))
}

/// Calculate content hash for a library unit from its files.
fn calculate_content_hash(library_unit_files: Vec<(QualifiedPath, Key)>) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let bytes = serde_json::to_vec(&LibraryUnitHash::new(library_unit_files))?;
    hasher.write_all(&bytes)?;
    hasher.finalize().to_hex().to_string().pipe(Ok)
}

/// Build a CargoSaveRequest from artifact data.
fn build_save_request(
    artifact: &BuiltArtifact,
    target: &str,
    content_hash: String,
    artifact_files: Vec<ArtifactFile>,
) -> CargoSaveRequest {
    CargoSaveRequest::builder()
        .package_name(&artifact.package_name)
        .package_version(&artifact.package_version)
        .target(target)
        .library_crate_compilation_unit_hash(&artifact.library_crate_compilation_unit_hash)
        .maybe_build_script_compilation_unit_hash(
            artifact.build_script_compilation_unit_hash.as_ref(),
        )
        .maybe_build_script_execution_unit_hash(artifact.build_script_execution_unit_hash.as_ref())
        .content_hash(content_hash)
        .artifacts(artifact_files)
        .build()
}
