use std::{collections::HashSet, io::Write, time::UNIX_EPOCH};

use color_eyre::{
    Result,
    eyre::{Context as _, OptionExt as _, bail},
};
use futures::{TryStreamExt as _, stream};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use tap::Pipe as _;
use tracing::{debug, instrument, trace, warn};

use crate::{
    cargo::{
        ArtifactKey, ArtifactPlan, BuildScriptOutput, BuiltArtifact, DepInfo, QualifiedPath,
        RootOutput, RustcTarget, Workspace,
    },
    cas::CourierCas,
    fs,
    path::{AbsFilePath, TryJoinWith as _},
};
use clients::{
    Courier,
    courier::v1::{
        Key,
        cache::{ArtifactFile, CargoSaveRequest},
    },
};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SaveProgress {
    pub uploaded_artifacts: u64,
    pub total_artifacts: u64,
    pub uploaded_files: u64,
    pub uploaded_bytes: u64,
}

#[instrument(skip(artifact_plan, on_progress))]
pub async fn save_artifacts(
    courier: &Courier,
    cas: &CourierCas,
    ws: &Workspace,
    artifact_plan: &ArtifactPlan,
    skip_artifacts: &HashSet<ArtifactKey>,
    skip_objects: &HashSet<Key>,
    mut on_progress: impl FnMut(&SaveProgress),
) -> Result<()> {
    trace!(?artifact_plan, "artifact plan");

    let mut progress = SaveProgress {
        uploaded_artifacts: 0,
        total_artifacts: artifact_plan.artifacts.len() as u64,
        uploaded_files: 0,
        uploaded_bytes: 0,
    };

    for artifact_key in &artifact_plan.artifacts {
        let artifact = BuiltArtifact::from_key(ws, artifact_key.clone()).await?;
        debug!(?artifact, "caching artifact");

        if skip_artifacts.contains(artifact_key) {
            trace!(
                ?artifact_key,
                "skipping backup: artifact was restored from cache"
            );
            progress.total_artifacts -= 1;
            continue;
        }

        let lib_files = collect_library_files(&artifact).await?;
        let build_script_files = collect_build_script_files(ws, &artifact).await?;
        let files_to_save = lib_files.into_iter().chain(build_script_files).collect();
        let (library_unit_files, artifact_files, bulk_entries) =
            process_files_for_upload(ws, files_to_save, skip_objects).await?;

        let (bytes, files) = upload_files_bulk(cas, bulk_entries).await?;
        progress.uploaded_bytes += bytes;
        progress.uploaded_files += files;

        let content_hash = calculate_content_hash(library_unit_files)?;
        debug!(?content_hash, "calculated content hash");

        let request = build_save_request(
            &artifact,
            &artifact_plan.target,
            content_hash,
            artifact_files,
        );

        courier.cargo_cache_save(request).await?;
        progress.uploaded_artifacts += 1;
        on_progress(&progress);
    }

    Result::<_>::Ok(())
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
            let parsed = RootOutput::from_file(ws, &RustcTarget::ImplicitHost, path).await?;
            serde_json::to_vec(&parsed).context("serialize RootOutput")
        }
        Some("build-script-output") => {
            trace!(?path, "rewriting build-script-output file");
            let parsed = BuildScriptOutput::from_file(ws, &RustcTarget::ImplicitHost, path).await?;
            serde_json::to_vec(&parsed).context("serialize BuildScriptOutput")
        }
        Some("dep-info") => {
            trace!(?path, "rewriting dep-info file");
            let parsed = DepInfo::from_file(ws, &RustcTarget::ImplicitHost, path).await?;
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
async fn collect_library_files(artifact: &BuiltArtifact) -> Result<Vec<AbsFilePath>> {
    let lib_fingerprint_dir = artifact.profile_dir().try_join_dirs(&[
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

    // Build scripts are always stored in the base workspace profile directory,
    // whether cross compiling or not.
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

    // Outputs are either stored in the base workspace profile directory (if not
    // cross compiling) or are stored inside their specified target folder (if we
    // are).
    let output_fingerprint_dir = artifact.profile_dir().try_join_dirs(&[
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
        let qualified =
            QualifiedPath::parse(ws, &RustcTarget::ImplicitHost, &path.as_ref()).await?;

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

/// A content hash of a library unit's artifacts.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize)]
pub struct LibraryUnitHash {
    files: Vec<(QualifiedPath, Key)>,
}

/// A newtype wrapper for QualifiedPaths that provides an arbitrary but stable
/// Ord instance.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
struct LibraryUnitHashOrd<'a>(&'a QualifiedPath);

impl<'a> LibraryUnitHashOrd<'a> {
    fn discriminant(&self) -> u64 {
        match &self.0 {
            QualifiedPath::Rootless(_) => 0,
            QualifiedPath::RelativeTargetProfile(_) => 1,
            QualifiedPath::RelativeCargoHome(_) => 2,
            QualifiedPath::Absolute(_) => 3,
        }
    }
}

impl<'a> Ord for LibraryUnitHashOrd<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.0, &other.0) {
            (QualifiedPath::Rootless(a), QualifiedPath::Rootless(b)) => a.cmp(b),
            (QualifiedPath::RelativeTargetProfile(a), QualifiedPath::RelativeTargetProfile(b)) => {
                a.cmp(b)
            }
            (QualifiedPath::RelativeCargoHome(a), QualifiedPath::RelativeCargoHome(b)) => a.cmp(b),
            (QualifiedPath::Absolute(a), QualifiedPath::Absolute(b)) => a.cmp(b),
            (_, _) => self.discriminant().cmp(&other.discriminant()),
        }
    }
}

impl<'a> PartialOrd for LibraryUnitHashOrd<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LibraryUnitHash {
    /// Construct a library unit hash out of the files in the library unit.
    ///
    /// This constructor always ensures that the files are sorted, so any two
    /// sets of files with the same paths and contents will produce the same
    /// hash.
    pub fn new(mut files: Vec<(QualifiedPath, Key)>) -> Self {
        files.sort_by(|(q1, k1), (q2, k2)| {
            (LibraryUnitHashOrd(q1), k1).cmp(&(LibraryUnitHashOrd(q2), k2))
        });
        Self { files }
    }
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
