use std::{env::VarError, process::Stdio, time::Duration};

use color_eyre::{Result, Section, SectionExt, eyre::Context as _};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, trace};
use url::Url;
use uuid::Uuid;

use crate::{
    cargo::{
        ArtifactPlan, BuildScriptCompilationUnitPlan, BuildScriptExecutionUnitPlan,
        BuildScriptOutput, DepInfo, Fingerprint, LibraryCrateUnitPlan, QualifiedPath, Workspace,
    },
    cas::CourierCas,
    daemon::{CargoUploadRequest, DaemonPaths},
    progress::TransferBar,
};
use clients::{Courier, Token};

mod restore;
mod save;

pub use restore::{Restored, restore_artifacts};
pub use save::{SaveProgress, save_artifacts};

#[derive(Debug, Clone)]
pub struct CargoCache {
    #[debug("{:?}", courier_url.as_str())]
    courier_url: Url,
    courier_token: Token,
    courier: Courier,
    cas: CourierCas,
    ws: Workspace,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open", skip(courier_token))]
    pub async fn open(courier_url: Url, courier_token: Token, ws: Workspace) -> Result<Self> {
        let courier = Courier::new(courier_url.clone(), courier_token.clone())?;
        courier.ping().await.context("ping courier service")?;
        let cas = CourierCas::new(courier.clone());
        Ok(Self {
            courier_url,
            courier_token,
            courier,
            cas,
            ws,
        })
    }

    #[instrument(name = "CargoCache::save", skip(artifact_plan, restored))]
    pub async fn save(&self, artifact_plan: ArtifactPlan, restored: Restored) -> Result<Uuid> {
        trace!(?artifact_plan, "artifact plan");
        let paths = DaemonPaths::initialize().await?;

        // Start daemon if it's not already running. If it is, try to read its context
        // file to get its url, which we need to know in order to communicate with it.
        let daemon = if let Some(daemon) = paths.daemon_running().await? {
            daemon
        } else {
            // TODO: Ideally we'd replace this with proper double-fork daemonization to
            // avoid the security and compatibility concerns here: someone could replace the
            // binary at this path in the time between when this binary launches and when it
            // re-launches itself as a daemon.
            let hurry_binary = std::env::current_exe().context("read current binary path")?;

            // Spawn self as a child and wait for the ready message on STDOUT.
            let mut cmd = tokio::process::Command::new(hurry_binary);
            cmd.arg("daemon")
                .arg("start")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // If `HURRY_LOG` is not set, set it to `debug` by default so the
            // logs are useful.
            if let Err(VarError::NotPresent) = std::env::var("HURRY_LOG") {
                cmd.env("HURRY_LOG", "debug");
            }

            cmd.spawn()?;

            // This value was chosen arbitrarily. Adjust as needed.
            const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
            tokio::time::timeout(DAEMON_STARTUP_TIMEOUT, async {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if let Some(daemon) = paths.daemon_running().await? {
                        break Result::<_>::Ok(daemon);
                    }
                }
            })
            .await
            .context("wait for daemon to start")??
        };

        // Connect to daemon HTTP server.
        let client = reqwest::Client::default();
        let endpoint = format!("http://{}/api/v0/cargo/upload", daemon.url);

        // Send upload request.
        let request_id = Uuid::new_v4();
        let request = CargoUploadRequest {
            request_id,
            courier_url: self.courier_url.clone(),
            courier_token: self.courier_token.clone(),
            ws: self.ws.clone(),
            artifact_plan,
            skip_artifacts: restored.artifacts.into_iter().collect(),
            skip_objects: restored.objects.into_iter().collect(),
        };
        trace!(?request, "submitting upload request");
        let response = client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("send upload request to daemon at: {endpoint}"))
            .with_section(|| format!("{daemon:?}").header("Daemon context:"))?;
        trace!(?response, "got upload response");

        Ok(request_id)
    }

    #[instrument(name = "CargoCache::restore", skip(artifact_plan, progress))]
    pub async fn restore(
        &self,
        artifact_plan: &ArtifactPlan,
        progress: &TransferBar,
    ) -> Result<Restored> {
        restore_artifacts(&self.courier, &self.cas, &self.ws, artifact_plan, progress).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum SavedUnit {
    LibraryCrate(LibraryFiles, LibraryCrateUnitPlan),
    BuildScriptCompilation(BuildScriptCompiledFiles, BuildScriptCompilationUnitPlan),
    BuildScriptExecution(BuildScriptOutputFiles, BuildScriptExecutionUnitPlan),
}

/// Libraries are usually associated with 7 files:
///
/// - 2 output files (an `.rmeta` and an `.rlib`)
/// - 1 rustc dep-info (`.d`) file in the `deps` folder
/// - 4 files in the fingerprint directory
///   - An `EncodedDepInfo` file
///   - A fingerprint hash
///   - A fingerprint JSON
///   - An invoked timestamp
///
/// Of these files, the fingerprint hash, fingerprint JSON, and invoked
/// timestamp are all reconstructed from fingerprint information during
/// restoration.
#[derive(Debug, Serialize, Deserialize)]
struct LibraryFiles {
    /// These files come from the build plan's `outputs` field.
    // TODO: Can we specify this even more narrowly (e.g. with an `rmeta` and
    // `rlib` field)? I know there are other possible output files (e.g. `.so`
    // for proc macros on Linux and `.dylib` for something on macOS), but I
    // don't know what the enumerated list is.
    output_files: Vec<SavedFile>,
    /// This file is always at a known path in
    /// `deps/{package_name}-{unit_hash}.d`.
    dep_info_file: DepInfo,
    /// This information is parsed from the initial fingerprint created after
    /// the build, and is used to dynamically reconstruct fingerprints on
    /// restoration.
    fingerprint: Fingerprint,
    /// This file is always at a known path in
    /// `.fingerprint/{package_name}-{unit_hash}/dep-lib-{crate_name}`. It can
    /// be safely relocatably copied because the `EncodedDepInfo` struct only
    /// ever contains relative file path information (note that deps always have
    /// a `DepInfoPathType`, which is either `PackageRootRelative` or
    /// `BuildRootRelative`)[^1].
    ///
    /// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/dep_info.rs#L112
    encoded_dep_info_file: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BuildScriptCompiledFiles {
    /// This field contains the contents of the compiled build script program at
    /// `build_script_{build_script_entrypoint}-{build_script_compilation_unit_hash}`
    /// and hard linked at `build-script-{build_script_entrypoint}`.
    ///
    /// We need both of these files: the hard link is the file that's actually
    /// executed in the build plan, but the full path with the unit hash is the
    /// file that's tracked by the fingerprint.
    compiled_program: Vec<u8>,
    /// This is the path to the rustc dep-info file in the build directory.
    dep_info_file: DepInfo,
    /// This fingerprint is stored in `.fingerprint`, and is used to derive the
    /// timestamp, fingerprint hash file, and fingerprint JSON file.
    fingerprint: Fingerprint,
    /// This `EncodedDepInfo` (i.e. Cargo dep-info) file is stored in
    /// `.fingerprint`, and is directly saved and restored.
    encoded_dep_info_file: Vec<u8>,
}

// Note that we don't save
// `{profile_dir}/.fingerprint/{package_name}-{unit_hash}/root-output` because
// it is fully reconstructible from the workspace and the unit plan.
#[derive(Debug, Serialize, Deserialize)]
struct BuildScriptOutputFiles {
    out_dir_files: Vec<SavedFile>,
    stdout: BuildScriptOutput,
    stderr: Vec<u8>,
    fingerprint: Fingerprint,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedFile {
    path: QualifiedPath,
    contents: Vec<u8>,
    executable: bool,
}
