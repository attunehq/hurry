use std::{
    collections::HashMap,
    fmt::Debug,
    io::Write,
    path::PathBuf,
    time::{Duration, UNIX_EPOCH},
};

use cargo_metadata::TargetKind;
use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt, bail, eyre},
};
use futures::TryStreamExt as _;
use itertools::Itertools;
use serde::Serialize;
use tap::Pipe as _;
use tokio::task::JoinSet;
use tracing::{debug, instrument, trace, warn};

use crate::{
    Locked,
    cargo::{
        self, BuildPlan, BuildScriptOutput, CargoBuildArguments, CargoCompileMode, DepInfo,
        Profile, ProfileDir, QualifiedPath, RootOutput, RustcMetadata, Workspace,
    },
    cas::CourierCas,
    client::{ArtifactFile, CargoRestoreRequest, CargoSaveRequest, Courier},
    fs,
    hash::Blake3,
    path::{AbsDirPath, AbsFilePath, TryJoinWith as _},
};

#[derive(Debug, Clone)]
pub struct CargoCache {
    courier: Courier,
    cas: CourierCas,
    ws: Workspace,
}

impl CargoCache {
    #[instrument(name = "CargoCache::open")]
    pub async fn open(courier: Courier, ws: Workspace) -> Result<Self> {
        let cas = CourierCas::new(courier.clone());
        Ok(Self { cas, courier, ws })
    }

    #[instrument(name = "CargoCache::artifacts")]
    pub async fn artifact_plan(
        &self,
        profile: &Profile,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<ArtifactPlan> {
        let rustc = RustcMetadata::from_argv(&self.ws.root, &args)
            .await
            .context("parsing rustc metadata")?;
        trace!(?rustc, "rustc metadata");

        // Note that build plans as a feature are _deprecated_, although their
        // removal has not occurred in the last 6 years[^1]. If a stable
        // alternative comes along, we should migrate.
        //
        // An alternative is the `--unit-graph` flag, which is unstable but not
        // deprecated[^2]. Unfortunately, unit graphs do not provide information
        // about the `rustc` invocation argv or the unit hash of the build
        // script execution, both of which are necessary to construct the
        // artifact cache key. We could theoretically reconstruct this
        // information using the JSON build messages and RUSTC_WRAPPER
        // invocation recording, but that's way more work for no stronger of a
        // stability guarantee.
        //
        // [^1]: https://github.com/rust-lang/cargo/issues/7614
        // [^2]: https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph

        // From testing locally, it doesn't seem to matter in which order we
        // pass the flags but we pass the user flags first just in case as that
        // seems like it'd follow the principle of least surprise if ordering
        // ever does matter.
        //
        // FIXME: Why does running this clear all the compiled artifacts from
        // the target folder?
        let mut build_args = args.as_ref().to_argv();
        build_args.extend([
            String::from("--build-plan"),
            String::from("-Z"),
            String::from("unstable-options"),
        ]);

        let build_plan = cargo::invoke_output("build", build_args, [("RUSTC_BOOTSTRAP", "1")])
            .await?
            .pipe(|output| serde_json::from_slice::<BuildPlan>(&output.stdout))
            .context("parsing build plan")?;
        trace!(?build_plan, "build plan");

        let mut build_script_index_to_dir = HashMap::new();
        let mut build_script_program_file_to_index = HashMap::new();
        let mut build_script_executions = HashMap::new();
        let mut artifacts = Vec::new();
        for (i, invocation) in build_plan.invocations.iter().cloned().enumerate() {
            trace!(?invocation, "build plan invocation");

            // For each invocation, figure out what kind it is:
            // 1. Compiling a build script.
            // 2. Running a build script.
            // 3. Compiling a dependency.
            // 4. Compiling first-party code (which we skip for caching).
            if invocation.target_kind == [TargetKind::CustomBuild] {
                match invocation.compile_mode {
                    CargoCompileMode::Build => {
                        if let Some(output_file) = invocation.outputs.first() {
                            // For build script compilation, we need to know the
                            // directory into which the build script is
                            // compiled and record the compiled program file.

                            // First, we determine the build script compilation
                            // directory.
                            let output_file = PathBuf::from(output_file);
                            let out_dir = output_file
                                .parent()
                                .ok_or_eyre(
                                    "build script output file should have parent directory",
                                )?
                                .to_owned();
                            build_script_index_to_dir.insert(i, out_dir);

                            // Second, we record the executable program.
                            for file in invocation.outputs {
                                build_script_program_file_to_index.insert(file, i);
                            }
                            for (fslink, _orig) in invocation.links {
                                build_script_program_file_to_index.insert(fslink, i);
                            }
                        } else {
                            bail!(
                                "build script compilation produced no outputs: {:?}",
                                invocation
                            );
                        }
                    }
                    CargoCompileMode::RunCustomBuild => {
                        // For build script execution, we need to know which
                        // compiled build script is being executed, and where
                        // its outputs are being written.

                        // First, we need to figure out the build script being
                        // executed. We can do this using the program file being
                        // executed.
                        let build_script_index = *build_script_program_file_to_index
                            .get(&invocation.program)
                            .ok_or_eyre("build script should be compiled before execution")?;

                        // Second, we need to determine where its outputs are being written.
                        let out_dir = invocation
                            .env
                            .get("OUT_DIR")
                            .ok_or_eyre("build script execution should set OUT_DIR")?
                            .clone();

                        build_script_executions.insert(i, (build_script_index, out_dir));
                    }
                    _ => bail!(
                        "unknown compile mode for build script: {:?}",
                        invocation.compile_mode
                    ),
                }
            } else if invocation.target_kind == [TargetKind::Bin] {
                // Binaries are _always_ first-party code. Do nothing for now.
                continue;
            } else if invocation.target_kind.contains(&TargetKind::Lib)
                || invocation.target_kind.contains(&TargetKind::RLib)
                || invocation.target_kind.contains(&TargetKind::CDyLib)
                || invocation.target_kind.contains(&TargetKind::ProcMacro)
            {
                // Skip first-party workspace members. We only cache third-party dependencies.
                // `CARGO_PRIMARY_PACKAGE` is set if the user specifically requested the item
                // to be built; while it's technically possible for the user to do so for a
                // third-party dependency that's relatively rare (and arguably if they're asking
                // to compile it specifically, it _should_ probably be exempt from cache).
                let primary = invocation.env.get("CARGO_PRIMARY_PACKAGE");
                if primary.map(|v| v.as_str()) == Some("1") {
                    trace!(?invocation, "skipping: first party workspace member");
                    continue;
                }

                // Sanity check: everything here should be a dependency being compiled.
                if invocation.compile_mode != CargoCompileMode::Build {
                    bail!(
                        "unknown compile mode for dependency: {:?}",
                        invocation.compile_mode
                    );
                }

                let mut build_script_execution_index = None;
                for dep_index in &invocation.deps {
                    let dep = &build_plan.invocations[*dep_index];
                    // This should be sufficient to determine which dependency
                    // is the execution of the build script of the current
                    // library. There might be other build scripts for the same
                    // name and version (but different features), but they won't
                    // be listed as a `dep`.
                    if dep.target_kind == [TargetKind::CustomBuild]
                        && dep.compile_mode == CargoCompileMode::RunCustomBuild
                        && dep.package_name == invocation.package_name
                        && dep.package_version == invocation.package_version
                    {
                        build_script_execution_index = Some(dep_index);
                        break;
                    }
                }

                let lib_files: Vec<AbsFilePath> = invocation
                    .outputs
                    .into_iter()
                    .map(|f| AbsFilePath::try_from(f).context("parsing build plan output file"))
                    .collect::<Result<Vec<_>>>()?;
                let library_crate_compilation_unit_hash = {
                    let compiled_file = lib_files.first().ok_or_eyre("no compiled files")?;
                    let filename = compiled_file
                        .file_name()
                        .ok_or_eyre("no filename")?
                        .to_string_lossy();
                    let filename = filename.split_once('.').ok_or_eyre("no extension")?.0;

                    filename
                        .rsplit_once('-')
                        .ok_or_else(|| {
                            eyre!(
                                "no unit hash suffix in filename: {filename} (all files: {lib_files:?})"
                            )
                        })?
                        .1
                        .to_string()
                };
                let build_script = match build_script_execution_index {
                    Some(build_script_execution_index) => {
                        let (build_script_index, build_script_output_dir) = build_script_executions
                            .get(build_script_execution_index)
                            .ok_or_eyre(
                                "build script execution should have recorded output directory",
                            )?;
                        // We take the parent because this is always the `/out`
                        // folder of the build script.
                        let build_script_output_dir =
                            AbsDirPath::try_from(build_script_output_dir)?
                                .parent()
                                .ok_or_eyre("build script output directory has no parent")?;
                        let build_script_compiled_dir = build_script_index_to_dir
                            .get(build_script_index)
                            .ok_or_eyre(
                                "build script index should have recorded compilation directory",
                            )?;
                        let build_script_compiled_dir =
                            AbsDirPath::try_from(build_script_compiled_dir)?;
                        let build_script_compilation_unit_hash = {
                            let filename = &build_script_compiled_dir
                                .file_name()
                                .ok_or_eyre("no filename")?
                                .to_string_lossy();

                            filename
                                .rsplit_once('-')
                                .ok_or_eyre("no unit hash suffix")?
                                .1
                                .to_string()
                        };
                        let build_script_output_unit_hash = {
                            let filename = &build_script_output_dir
                                .file_name()
                                .ok_or_eyre("out_dir has no filename")?
                                .to_string_lossy();

                            filename
                                .rsplit_once('-')
                                .ok_or_eyre("no unit hash suffix")?
                                .1
                                .to_string()
                        };
                        Some((
                            BuildScriptDirs {
                                compiled_dir: build_script_compiled_dir,
                                output_dir: build_script_output_dir,
                            },
                            build_script_compilation_unit_hash,
                            build_script_output_unit_hash,
                        ))
                    }
                    None => None,
                };
                let (
                    build_script_files,
                    build_script_compilation_unit_hash,
                    build_script_execution_unit_hash,
                ) = match build_script {
                    Some((
                        build_script_files,
                        build_script_compilation_unit_hash,
                        build_script_execution_unit_hash,
                    )) => (
                        Some(build_script_files),
                        Some(build_script_compilation_unit_hash),
                        Some(build_script_execution_unit_hash),
                    ),
                    None => (None, None, None),
                };

                // Given a dependency being compiled, we need to determine the
                // compiled files, its build script directory, and its build
                // script outputs directory. These are the files that we're
                // going to save for this artifact.
                debug!(
                    compiled = ?lib_files,
                    build_script = ?build_script_files,
                    deps = ?invocation.deps,
                    "artifacts to save"
                );
                artifacts.push(ArtifactKey {
                    package_name: invocation.package_name,
                    package_version: invocation.package_version,
                    lib_files,
                    build_script_files,
                    library_crate_compilation_unit_hash,
                    build_script_compilation_unit_hash,
                    build_script_execution_unit_hash,
                });

                // TODO: If needed, we could try to read previous build script
                // output from the target directory here to try and supplement
                // information for built crates. I can't imagine why we would
                // need to do that, though.
            } else {
                bail!("unknown target kind: {:?}", invocation.target_kind);
            }
        }

        Ok(ArtifactPlan {
            artifacts,
            // TODO: We assume it's the same target as the host, but we really
            // should be parsing this from the `rustc` invocation.
            //
            // TODO: Is it possible for different artifacts in the same build to
            // have different targets?
            target: rustc.host_target.clone(),
            profile: profile.clone(),
        })
    }

    #[instrument(name = "CargoCache::save")]
    pub async fn save(&self, artifact: BuiltArtifact) -> Result<()> {
        // TODO: We should probably not be re-locking and unlocking on a per-artifact
        // basis. Maybe this method should instead take a Vec?
        let profile_dir = self.ws.open_profile_locked(&artifact.profile).await?;

        // Determine which files will be saved.
        let lib_files = {
            let lib_fingerprint_dir = profile_dir.root().try_join_dirs(&[
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
                let compiled_fingerprint_dir = profile_dir.root().try_join_dirs(&[
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
                let output_fingerprint_dir = profile_dir.root().try_join_dirs(&[
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

        // For each file, save it into the CAS and calculate its key.
        //
        // TODO: Fuse this operation with the loop above where we discover the
        // needed files? Would that give better performance?
        let mut library_unit_files = vec![];
        let mut artifact_files = vec![];
        for path in files_to_save {
            match fs::read_buffered(&path).await? {
                Some(content) => {
                    let content = Self::rewrite(&profile_dir, &path, &content).await?;

                    let key = self.cas.store(&content).await?;
                    debug!(?path, ?key, "stored object");

                    // Gather metadata for the artifact file.
                    let metadata = fs::Metadata::from_file(&path)
                        .await?
                        .ok_or_eyre("could not stat file metadata")?;
                    let mtime_nanos = metadata.mtime.duration_since(UNIX_EPOCH)?.as_nanos();
                    let portable = QualifiedPath::parse(&profile_dir, path.as_std_path()).await?;

                    artifact_files.push(
                        ArtifactFile::builder()
                            .object_key(key.to_string())
                            .path(portable.clone())
                            .mtime_nanos(mtime_nanos)
                            .executable(metadata.executable)
                            .build(),
                    );

                    library_unit_files.push((portable, key));
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

        // Calculate the content hash.
        let content_hash = {
            let mut hasher = blake3::Hasher::new();
            let bytes = serde_json::to_vec(&LibraryUnitHash::new(library_unit_files.clone()))?;
            hasher.write_all(&bytes)?;
            hasher.finalize().to_hex().to_string()
        };
        debug!(?content_hash, "calculated content hash");

        // Save the library unit via the Courier API.
        let request = CargoSaveRequest::builder()
            .package_name(artifact.package_name)
            .package_version(artifact.package_version)
            .target(artifact.target)
            .library_crate_compilation_unit_hash(artifact.library_crate_compilation_unit_hash)
            .maybe_build_script_compilation_unit_hash(artifact.build_script_compilation_unit_hash)
            .maybe_build_script_execution_unit_hash(artifact.build_script_execution_unit_hash)
            .content_hash(content_hash)
            .artifacts(artifact_files)
            .build();

        self.courier.cargo_cache_save(request).await
    }

    #[instrument(name = "CargoCache::restore")]
    pub async fn restore(&self, artifact_plan: &ArtifactPlan) -> Result<()> {
        debug!("start restoring");

        // Open the profile dir once to extract owned paths upfront.
        // We do this because `ProfileDir` has a lifetime for the lock, so we can't send
        // it to worker tasks; this is valid so long as we actually hold `ProfileDir`
        // for the entire time the task workers live (which we do).
        let profile_dir = self.ws.open_profile_locked(&artifact_plan.profile).await?;
        let profile_root = profile_dir.root().to_owned();
        let cargo_home = profile_dir.workspace.cargo_home.clone();

        // TODO: We should probably make this concurrent on something else since this is
        // likely going to be primarily blocked on network transfer.
        let worker_count = num_cpus::get();

        // Normally I don't like buffered channels, but here we let it buffer so that we
        // can potentially start working on the next artifact_plan while the prior is
        // still being worked on. We want to avoid uneccesary memory overhead but each
        // individual artifact file is relatively small so we'd rather trade some space
        // for latency mitigation here.
        let (tx, rx) = flume::bounded::<ArtifactFile>(worker_count * 10);
        let mut cas_restore_workers = JoinSet::new();
        for _ in 0..worker_count {
            let rx = rx.clone();
            let cas = self.cas.clone();
            let profile_root = profile_root.clone();
            let cargo_home = cargo_home.clone();
            cas_restore_workers.spawn(async move {
                let restore = async move |artifact: &ArtifactFile| -> Result<()> {
                    let key = Blake3::from_hex_string(&artifact.object_key)?;

                    // Reconstruct the portable path to an absolute path for this machine.
                    let path = artifact
                        .path
                        .reconstruct_raw(&profile_root, &cargo_home)
                        .pipe(AbsFilePath::try_from)?;

                    // Get the data from the CAS and reconstruct it according to the local machine.
                    let data = cas.must_get(&key).await?;
                    let data = Self::reconstruct(&profile_root, &cargo_home, &path, &data).await?;

                    // Write the file.
                    let mtime = UNIX_EPOCH + Duration::from_nanos(artifact.mtime_nanos as u64);
                    let metadata = fs::Metadata::builder()
                        .mtime(mtime)
                        .executable(artifact.executable)
                        .build();
                    fs::write(&path, &data).await?;
                    metadata.set_file(&path).await?;
                    Result::<()>::Ok(())
                };

                while let Ok(artifact) = rx.recv_async().await {
                    if let Err(error) = restore(&artifact).await {
                        warn!(?error, ?artifact, "failed to restore file");
                    }
                }
            });
        }

        for artifact in &artifact_plan.artifacts {
            let request = CargoRestoreRequest::builder()
                .package_name(&artifact.package_name)
                .package_version(&artifact.package_version)
                .target(&artifact_plan.target)
                .library_crate_compilation_unit_hash(&artifact.library_crate_compilation_unit_hash)
                .maybe_build_script_compilation_unit_hash(
                    artifact.build_script_compilation_unit_hash.as_ref(),
                )
                .maybe_build_script_execution_unit_hash(
                    artifact.build_script_execution_unit_hash.as_ref(),
                )
                .build();

            let Some(response) = self.courier.cargo_cache_restore(request).await? else {
                debug!(?artifact, "no matching library unit build found");
                continue;
            };

            for artifact_file in response.artifacts {
                if let Err(error) = tx.send_async(artifact_file).await {
                    panic!("invariant violated: no restore workers are alive: {error:?}");
                }
            }
        }

        // The channels are done being used here, so we drop them; the workers may still
        // be using their clones if they're in process writing files but they'll keep
        // rx alive until they're all done.
        drop(rx);
        drop(tx);
        while let Some(worker) = cas_restore_workers.join_next().await {
            worker.context("cas restore worker")?;
        }

        debug!("done restoring");
        Ok(())
    }

    /// Rewrite file contents before storing in CAS to normalize paths.
    #[instrument(name = "CargoCache::rewrite_for_storage", skip(content))]
    async fn rewrite(
        profile_dir: &ProfileDir<'_, Locked>,
        path: &AbsFilePath,
        content: &[u8],
    ) -> Result<Vec<u8>> {
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
                let parsed = RootOutput::from_file(profile_dir, path).await?;
                serde_json::to_vec(&parsed).context("serialize RootOutput")
            }
            Some("build-script-output") => {
                trace!(?path, "rewriting build-script-output file");
                let parsed = BuildScriptOutput::from_file(profile_dir, path).await?;
                serde_json::to_vec(&parsed).context("serialize BuildScriptOutput")
            }
            Some("dep-info") => {
                trace!(?path, "rewriting dep-info file");
                let parsed = DepInfo::from_file(profile_dir, path).await?;
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

    /// Reconstruct file contents after retrieving from CAS.
    #[instrument(name = "CargoCache::reconstruct_from_storage", skip(content))]
    async fn reconstruct(
        profile_root: &AbsDirPath,
        cargo_home: &AbsDirPath,
        path: &AbsFilePath,
        content: &[u8],
    ) -> Result<Vec<u8>> {
        use itertools::Itertools as _;

        // Determine what kind of file this is based on path structure.
        let components = path.component_strs_lossy().collect::<Vec<_>>();

        // Look at the last few components to determine file type.
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
                trace!(?path, "reconstructing root-output file");
                let parsed = serde_json::from_slice::<RootOutput>(content)?;
                Ok(parsed
                    .reconstruct_raw(profile_root, cargo_home)
                    .into_bytes())
            }
            Some("build-script-output") => {
                trace!(?path, "reconstructing build-script-output file");
                let parsed = serde_json::from_slice::<BuildScriptOutput>(content)?;
                Ok(parsed
                    .reconstruct_raw(profile_root, cargo_home)
                    .into_bytes())
            }
            Some("dep-info") => {
                trace!(?path, "reconstructing dep-info file");
                let parsed = serde_json::from_slice::<DepInfo>(content)?;
                Ok(parsed
                    .reconstruct_raw(profile_root, cargo_home)
                    .into_bytes())
            }
            None => {
                // No reconstruction needed, use as-is.
                Ok(content.to_vec())
            }
            Some(unknown) => {
                bail!("unknown file type for reconstruction: {unknown}")
            }
        }
    }
}

/// An ArtifactPlan represents the collection of information known about the
/// artifacts for a build statically at compile-time.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArtifactPlan {
    pub profile: Profile,
    pub target: String,

    pub artifacts: Vec<ArtifactKey>,
}

/// An ArtifactKey represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) statically
/// at plan-time.
///
/// In particular, this information does _not_ include information derived from
/// compiling and running the build script, such as `rustc` flags from build
/// script output directives.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArtifactKey {
    // Partial artifact key information. Note that this is only derived from the
    // build plan, and therefore is missing essential information (e.g. `rustc`
    // flags from build script output directives) that can only be determined
    // interactively.
    //
    // TODO: There are more fields here that we can know from the planning stage
    // that need to be added (e.g. features).
    package_name: String,
    package_version: String,

    // Artifact folders to save and restore.
    lib_files: Vec<AbsFilePath>,
    build_script_files: Option<BuildScriptDirs>,

    // Unit hashes.
    library_crate_compilation_unit_hash: String,
    build_script_compilation_unit_hash: Option<String>,
    build_script_execution_unit_hash: Option<String>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct BuildScriptDirs {
    compiled_dir: AbsDirPath,
    output_dir: AbsDirPath,
}

/// A BuiltArtifact represents the information known about a library unit (i.e.
/// a library crate, its build script, and its build script outputs) after it
/// has been built.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct BuiltArtifact {
    package_name: String,
    package_version: String,
    target: String,
    profile: Profile,

    lib_files: Vec<AbsFilePath>,
    build_script_files: Option<BuildScriptDirs>,

    library_crate_compilation_unit_hash: String,
    build_script_compilation_unit_hash: Option<String>,
    build_script_execution_unit_hash: Option<String>,
}

impl BuiltArtifact {
    /// Given an `ArtifactKey`, read the build script output directories on
    /// disk and construct a `BuiltArtifact`.
    #[instrument(name = "BuiltArtifact::from_key")]
    pub async fn from_key(key: ArtifactKey, target: String, profile: Profile) -> Result<Self> {
        // TODO: Read the build script output from the build folders, and parse
        // the output for directives. Use this to construct the rustc
        // invocation, and use all of this information to fully construct the
        // cache key.

        // FIXME: What we actually do right now is just copy fields and ignore
        // that dynamic fields might not be captured by the unit hash. This
        // behavior is incorrect! We are only ignoring this for now so we can
        // get something simple working end-to-end.
        Ok(BuiltArtifact {
            package_name: key.package_name,
            package_version: key.package_version,
            target,
            profile,

            lib_files: key.lib_files,
            build_script_files: key.build_script_files,

            library_crate_compilation_unit_hash: key.library_crate_compilation_unit_hash,
            build_script_compilation_unit_hash: key.build_script_compilation_unit_hash,
            build_script_execution_unit_hash: key.build_script_execution_unit_hash,
        })
    }
}

/// A content hash of a library unit's artifacts.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize)]
struct LibraryUnitHash {
    files: Vec<(QualifiedPath, Blake3)>,
}

impl LibraryUnitHash {
    /// Construct a library unit hash out of the files in the library unit.
    ///
    /// This constructor always ensures that the files are sorted, so any two
    /// sets of files with the same paths and contents will produce the same
    /// hash.
    fn new(mut files: Vec<(QualifiedPath, Blake3)>) -> Self {
        files.sort();
        Self { files }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq as pretty_assert_eq;

    #[tokio::test]
    async fn build_plan_flag_order_does_not_matter() {
        // This is a relatively basic test to start with; if we find other edge
        // cases we want to test we should add them here (or in a similar test).
        let user_args = ["--release"];
        let tool_args = ["--build-plan", "-Z", "unstable-options"];
        let env = [("RUSTC_BOOTSTRAP", "1")];
        let cmd = "build";

        let args = user_args.iter().chain(tool_args.iter());
        let user_args_first = match cargo::invoke_output(cmd, args, env).await {
            Ok(output) => output.stdout,
            Err(e) => panic!("user args first should succeed: {e}"),
        };

        let args = tool_args.iter().chain(user_args.iter());
        let tool_args_first = match cargo::invoke_output(cmd, args, env).await {
            Ok(output) => output.stdout,
            Err(e) => panic!("tool args first should succeed: {e}"),
        };

        let user_plan = serde_json::from_slice::<BuildPlan>(&user_args_first).unwrap();
        let tool_plan = serde_json::from_slice::<BuildPlan>(&tool_args_first).unwrap();
        pretty_assert_eq!(
            user_plan,
            tool_plan,
            "both orderings should produce same build plan"
        );
    }
}
