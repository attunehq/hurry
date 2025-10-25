use std::{collections::HashSet, hash::RandomState, io::Write, time::UNIX_EPOCH};

use axum::{Json, Router, extract::State, routing::post};
use clap::Parser;
use clients::{
    Courier,
    courier::v1::{
        Key,
        cache::{ArtifactFile, CargoSaveRequest},
    },
};
use color_eyre::{
    Result,
    eyre::{Context as _, Error, OptionExt as _, bail},
};
use derive_more::Debug;
use futures::{TryStreamExt as _, stream};
use hurry::{
    cargo::{
        ArtifactKey, ArtifactPlan, BuildScriptOutput, BuiltArtifact, DepInfo, LibraryUnitHash,
        QualifiedPath, RootOutput, Workspace,
    },
    cas::CourierCas,
    fs, mk_rel_file,
    path::{AbsFilePath, JoinWith, TryJoinWith},
};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tap::Pipe as _;
use tokio::net::UnixListener;
use tower_http::trace::TraceLayer;
use tracing::{debug, dispatcher, info, instrument, trace, warn};
use tracing_error::ErrorLayer;
use tracing_subscriber::{
    Layer as _, fmt::MakeWriter, layer::SubscriberExt as _, util::SubscriberInitExt,
};
use tracing_tree::time::Uptime;
use url::Url;

#[derive(Debug, Parser)]
pub struct Flags {
    /// Base URL for the Courier instance.
    #[arg(
        long = "hurry-courier-url",
        env = "HURRY_COURIER_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{courier_url}")]
    courier_url: Url,
}

fn make_logger<W>(writer: W) -> impl tracing::Subscriber
where
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    return tracing_subscriber::registry()
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
                .with_writer(writer)
                .with_targets(false)
                .with_filter(
                    tracing_subscriber::EnvFilter::builder()
                        .with_env_var("HURRYD_LOG")
                        .from_env_lossy(),
                ),
        );
}

#[instrument]
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize error handling.
    color_eyre::install()?;

    // Parse flags.
    let flags = Flags::parse();

    // Set up daemon directory.
    let cache_dir = hurry::fs::user_global_cache_path().await?;
    let pid = std::process::id();
    let socket_path = cache_dir.join(mk_rel_file!("hurryd.sock"));
    let pid_file_path = cache_dir.join(mk_rel_file!("hurryd.pid"));
    let stderr_file_path = cache_dir.try_join_file(format!("hurryd.{}.err", pid))?;
    dispatcher::with_default(&make_logger(std::io::stderr).into(), || {
        debug!(
            ?socket_path,
            ?pid_file_path,
            ?stderr_file_path,
            "file paths"
        );
        info!(?stderr_file_path, "logging to file");
    });

    // Initialize logging.
    make_logger(std::fs::File::create(stderr_file_path.as_std_path())?).init();

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

    let courier = Courier::new(flags.courier_url)?;
    let cas = CourierCas::new(courier.clone());
    let state = ServerState { cas, courier };

    let cargo = Router::new()
        .route("/upload", post(upload))
        .with_state(state);

    let app = Router::new()
        .nest("/api/v0/cargo", cargo)
        .layer(TraceLayer::new_for_http());
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Debug, Clone)]
struct ServerState {
    cas: CourierCas,
    courier: Courier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CargoUploadRequest {
    ws: Workspace,
    artifact_plan: ArtifactPlan,
    skip_artifacts: Vec<ArtifactKey>,
    skip_objects: Vec<Key>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CargoUploadResponse {
    ok: bool,
}

async fn upload(
    State(state): State<ServerState>,
    Json(req): Json<CargoUploadRequest>,
) -> Json<CargoUploadResponse> {
    let state = state.clone();
    tokio::spawn(async move {
        let restored_artifacts: HashSet<ArtifactKey, RandomState> =
            HashSet::from_iter(req.skip_artifacts);
        let restored_objects: HashSet<Key, RandomState> = HashSet::from_iter(req.skip_objects);

        let target_path = &req.ws.profile_dir;

        for artifact_key in req.artifact_plan.artifacts {
            let artifact = BuiltArtifact::from_key(&req.ws, artifact_key.clone()).await?;
            debug!(?artifact, "caching artifact");

            // Determine which files will be saved.
            let lib_files = {
                let lib_fingerprint_dir = target_path.try_join_dirs(&[
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
                    .into_iter()
                    .chain(lib_fingerprint_files)
                    .collect::<Vec<_>>()
            };
            let build_script_files = match artifact.build_script_files {
                Some(build_script_files) => {
                    let compiled_files = fs::walk_files(&build_script_files.compiled_dir)
                        .try_collect::<Vec<_>>()
                        .await?;
                    let compiled_fingerprint_dir = target_path.try_join_dirs(&[
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
                    let output_fingerprint_dir = target_path.try_join_dirs(&[
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
                        .collect()
                }
                None => vec![],
            };

            let files_to_save = lib_files.into_iter().chain(build_script_files);
            if restored_artifacts.contains(&artifact_key) {
                trace!(
                    ?artifact_key,
                    "skipping backup: artifact was restored from cache"
                );
                continue;
            }

            // For each file, save it into the CAS and calculate its key.
            //
            // TODO: Fuse this operation with the loop above where we discover the
            // needed files? Would that give better performance?
            let mut library_unit_files = Vec::<(QualifiedPath, Key)>::new();
            let mut artifact_files = Vec::<ArtifactFile>::new();
            let mut bulk_entries = Vec::<(Key, Vec<u8>, AbsFilePath)>::new();

            // First pass: read files, calculate keys, and collect entries for bulk upload.
            for path in files_to_save {
                match fs::read_buffered(&path).await? {
                    Some(content) => {
                        let content = rewrite(&req.ws, &path, &content).await?;
                        let key = Key::from_buffer(&content);

                        // Gather metadata for the artifact file.
                        let metadata = fs::Metadata::from_file(&path)
                            .await?
                            .ok_or_eyre("could not stat file metadata")?;
                        let mtime_nanos = metadata.mtime.duration_since(UNIX_EPOCH)?.as_nanos();
                        let qualified = QualifiedPath::parse(&req.ws, path.as_std_path()).await?;

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
                    None => {
                        // Note that this is not necessarily incorrect! For example,
                        // Cargo seems to claim to emit `.dwp` files for its `.so`s,
                        // but those don't seem to be there by the time the process
                        // actually finishes. I'm not sure if they're deleted or
                        // just never written.
                        warn!("failed to read file: {}", path);
                    }
                }
            }

            // Second pass: upload files using bulk operations.
            if !bulk_entries.is_empty() {
                debug!(count = bulk_entries.len(), "uploading files");

                // Store the actual entries.
                let result = bulk_entries
                    .iter()
                    .map(|(key, content, _)| (key.clone(), content.clone()))
                    .collect::<Vec<_>>()
                    .pipe(stream::iter)
                    .pipe(|stream| state.cas.store_bulk(stream))
                    .await
                    .context("upload batch")?;

                // Update statistics based on bulk result.
                for (key, _, path) in &bulk_entries {
                    if result.written.contains(key) {
                        debug!(?path, ?key, "uploaded via bulk");
                    } else if result.skipped.contains(key) {
                        debug!(?path, ?key, "skipped by server (already exists)");
                    }
                }

                // Log any errors but continue (partial success model).
                for error in &result.errors {
                    warn!(
                        key = ?error.key,
                        error = %error.error,
                        "failed to upload file in bulk operation"
                    );
                }
            }

            // Calculate the content hash.
            let content_hash = {
                let mut hasher = blake3::Hasher::new();
                let bytes = serde_json::to_vec(&LibraryUnitHash::new(library_unit_files))?;
                hasher.write_all(&bytes)?;
                hasher.finalize().to_hex().to_string()
            };
            debug!(?content_hash, "calculated content hash");

            // Save the library unit via the Courier API.
            let request = CargoSaveRequest::builder()
                .package_name(artifact.package_name)
                .package_version(artifact.package_version)
                .target(&req.artifact_plan.target)
                .library_crate_compilation_unit_hash(artifact.library_crate_compilation_unit_hash)
                .maybe_build_script_compilation_unit_hash(
                    artifact.build_script_compilation_unit_hash,
                )
                .maybe_build_script_execution_unit_hash(artifact.build_script_execution_unit_hash)
                .content_hash(content_hash)
                .artifacts(artifact_files)
                .build();

            state.courier.cargo_cache_save(request).await?;
        }
        Ok::<(), Error>(())
    });
    Json(CargoUploadResponse { ok: true })
}

#[instrument]
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
