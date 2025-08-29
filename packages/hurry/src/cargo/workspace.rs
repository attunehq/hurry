use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use bon::{Builder, bon};
use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    Result, Section, SectionExt,
    eyre::{Context, OptionExt, eyre},
};
use derive_more::{Debug, Display};
use hurry::{
    Locked, Unlocked,
    fs::{self, Index, LockFile},
    hash::Blake3,
};
use itertools::Itertools;
use location_macros::workspace_dir;
use relative_path::{PathExt, RelativePath, RelativePathBuf};
use serde::Deserialize;
use tap::{Pipe, TapFallible};
use tokio::task::spawn_blocking;
use tracing::{debug, instrument, trace};

use crate::{
    cache::Artifact,
    cargo::{Profile, read_argv},
};

/// A Cargo workspace.
///
/// Note that in Cargo, "workspace" projects are slightly different than
/// standard projects; however for `hurry` they are not.
#[derive(Debug, Display)]
#[display("{root}")]
pub struct Workspace {
    /// The root directory of the workspace.
    pub root: Utf8PathBuf,

    /// The target directory in the workspace.
    #[debug(skip)]
    pub target: Utf8PathBuf,

    /// Parsed `rustc` metadata relating to the current workspace.
    #[debug(skip)]
    pub rustc: RustcMetadata,

    /// Dependencies in the workspace, keyed by [`Dependency::key`].
    #[debug(skip)]
    pub dependencies: HashMap<Blake3, Dependency>,
}

impl Workspace {
    /// Parse metadata about the current workspace.
    ///
    /// "Current workspace" is discovered by parsing the arguments passed
    /// to `hurry` and using `--manifest_path` if it is available;
    /// if not then it uses the current working directory.
    //
    // TODO: A few of these setup steps could be parallelized...
    // I'm not certain they're worth the thread spawn cost
    // but this can be mitigated by using the rayon thread pool.
    #[instrument(name = "Workspace::from_argv")]
    pub async fn from_argv(argv: &[String]) -> Result<Self> {
        // TODO: Maybe we should just replicate this logic and perform it
        // statically using filesystem operations instead of shelling out? This
        // costs something on the order of 200ms, which is not _terrible_ but
        // feels much slower than if we just did our own filesystem reads.
        let manifest_path = read_argv(argv, "--manifest-path").map(String::from);
        let metadata = spawn_blocking(move || -> Result<_> {
            let mut cmd = cargo_metadata::MetadataCommand::new();
            if let Some(p) = manifest_path {
                cmd.manifest_path(p);
            }
            let metadata = cmd.exec().context("could not read cargo metadata")?;
            debug!(?metadata, "cargo metadata");
            Ok(metadata)
        })
        .await
        .context("join task")?
        .context("read cargo metadata")?;

        // TODO: This currently blows up if we have no lockfile.
        let cargo_lock = metadata.workspace_root.join("Cargo.lock");
        let lockfile = spawn_blocking(move || -> Result<_> {
            let lockfile = cargo_lock::Lockfile::load(cargo_lock).context("load cargo lockfile")?;
            debug!(?lockfile, "cargo lockfile");
            Ok(lockfile)
        })
        .await
        .context("join task")?
        .context("read cargo lockfile")?;

        let rustc_meta = RustcMetadata::from_argv(&metadata.workspace_root, argv)
            .await
            .context("read rustc metadata")?;
        debug!(?rustc_meta, "rustc metadata");

        // We only care about third party packages for now.
        //
        // From observation, first party packages seem to have
        // no `source` or `checksum` while third party packages do,
        // so we just filter anything that doesn't have these.
        //
        // In addition, to keep things simple, we filter to only
        // packages that are in the default registry.
        //
        // Only dependencies reported here are actually cached;
        // anything we exclude here is ignored by the caching system.
        //
        // TODO: Support caching packages not in the default registry.
        // TODO: Support caching first party packages.
        // TODO: Support caching git etc packages.
        // TODO: How can we properly report `target` for cross compiled deps?
        let dependencies = lockfile
            .packages
            .into_iter()
            .filter_map(|package| match (&package.source, &package.checksum) {
                (Some(source), Some(checksum)) if source.is_default_registry() => {
                    Dependency::builder()
                        .checksum(checksum.to_string())
                        .name(package.name.to_string())
                        .version(package.version.to_string())
                        .target(&rustc_meta.llvm_target)
                        .build()
                        .pipe(Some)
                }
                _ => {
                    trace!(?package, "skipped indexing package for cache");
                    None
                }
            })
            .map(|dependency| (dependency.key(), dependency))
            .inspect(|(key, dependency)| trace!(?key, ?dependency, "indexed dependency"))
            .collect::<HashMap<_, _>>();

        Ok(Self {
            root: metadata.workspace_root,
            target: metadata.target_directory,
            rustc: rustc_meta,
            dependencies,
        })
    }

    /// Ensure that the workspace `target/` directory
    /// is created and well formed with the provided
    /// profile directory created.
    #[instrument(name = "Workspace::init_target")]
    pub async fn init_target(&self, profile: &Profile) -> Result<()> {
        const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";
        const CACHEDIR_TAG_CONTENT: &[u8] =
            include_bytes!(concat!(workspace_dir!(), "/static/cargo/CACHEDIR.TAG"));

        // TODO: do we need to create `.rustc_info.json` to get cargo
        // to recognize the target folder as valid when restoring caches?
        fs::create_dir_all(self.target.join(profile.as_str()))
            .await
            .context("create target directory")?;
        fs::write(self.target.join(CACHEDIR_TAG_NAME), CACHEDIR_TAG_CONTENT)
            .await
            .context("write CACHEDIR.TAG")
    }

    /// Open the given named profile directory in the workspace.
    pub async fn open_profile(&self, profile: &Profile) -> Result<ProfileDir<'_, Unlocked>> {
        ProfileDir::open(self, profile).await
    }

    /// Open the given named profile directory in the workspace locked.
    pub async fn open_profile_locked(&self, profile: &Profile) -> Result<ProfileDir<'_, Locked>> {
        self.open_profile(profile)
            .await
            .context("open profile")?
            .pipe(|target| target.lock())
            .await
            .context("lock profile")
    }
}

/// A Cargo dependency.
///
/// This isn't the full set of information about a dependency, but it's enough
/// to identify it uniquely within a workspace for the purposes of caching.
///
/// Each piece of data in this struct is used to build the "cache key"
/// for the dependency; the intention is that each dependency is cached
/// independently and restored in other projects based on a matching
/// cache key derived from other instances of `hurry` reading the
/// `Cargo.lock` and other workspace/compiler/platform metadata.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Builder)]
#[display("{name}@{version}")]
pub struct Dependency {
    /// The name of the dependency.
    #[builder(into)]
    pub name: String,

    /// The version of the dependency.
    #[builder(into)]
    pub version: String,

    /// The checksum of the dependency.
    #[builder(into)]
    pub checksum: String,

    /// The target triple for which the dependency
    /// is being or has been built.
    ///
    /// Examples:
    /// ```not_rust
    /// aarch64-apple-darwin
    /// x86_64-unknown-linux-gnu
    /// ```
    #[builder(into)]
    pub target: String,
}

impl Dependency {
    /// Hash key for the dependency.
    pub fn key(&self) -> Blake3 {
        Self::key_for()
            .checksum(&self.checksum)
            .name(&self.name)
            .target(&self.target)
            .version(&self.version)
            .call()
    }
}

#[bon]
impl Dependency {
    /// Produce a hash key for all the fields of a dependency
    /// without having to actually make a dependency instance
    /// (which may involve cloning).
    #[builder]
    pub fn key_for(
        name: impl AsRef<[u8]>,
        version: impl AsRef<[u8]>,
        checksum: impl AsRef<[u8]>,
        target: impl AsRef<[u8]>,
    ) -> Blake3 {
        let name = name.as_ref();
        let version = version.as_ref();
        let checksum = checksum.as_ref();
        let target = target.as_ref();
        Blake3::from_fields([name, version, checksum, target])
    }
}

/// A profile directory inside a [`Workspace`].
#[derive(Debug, Clone)]
pub struct ProfileDir<'ws, State> {
    #[debug(skip)]
    state: PhantomData<State>,

    /// The lockfile for the directory.
    ///
    /// The intention of this lock is to prevent multiple `hurry` _or `cargo`_
    /// instances from mutating the state of the directory at the same time,
    /// or from mutating it at the same time as another instance
    /// is reading it.
    ///
    /// This lockfile uses the same name and implementation as `cargo` uses,
    /// so a locked `ProfileDir` in `hurry` will block `cargo` and vice versa.
    #[debug(skip)]
    lock: LockFile<State>,

    /// The workspace in which this build profile is located.
    pub workspace: &'ws Workspace,

    /// The index of files inside the profile directory.
    /// Paths in this index are relative to [`ProfileDir::root`].
    ///
    /// This index is built when the profile directory is locked.
    /// Currently there's no explicit unlock mechanism for profiles since
    /// they're just dropped, but if we ever add one that's where we'd clear
    /// this and set it to `None`.
    ///
    /// This is in an `Arc` so that we don't have to clone the whole index
    /// when we clone the `ProfileDir`.
    index: Option<Arc<Index>>,

    /// The root of the directory,
    /// relative to [`workspace.target`](Workspace::target).
    ///
    /// Note: this is intentionally not `pub` because we only want to give
    /// callers access to the directory when the cache is locked;
    /// reference the `root` method in the locked implementation block.
    /// The intention here is to minimize the chance of callers mutating or
    /// referencing the contents of the cache while it is locked.
    root: RelativePathBuf,

    /// The profile to which this directory refers.
    ///
    /// By default, profiles are `release`, `debug`, `test`, and `bench`
    /// although users can also define custom profiles, which is why
    /// this value is an opaque string:
    /// https://doc.rust-lang.org/cargo/reference/profiles.html#custom-profiles
    pub profile: Profile,
}

impl<'ws> ProfileDir<'ws, Unlocked> {
    /// Instantiate a new instance for the provided profile in the workspace.
    /// If the directory doesn't already exist, it is created.
    #[instrument(name = "ProfileDir::open")]
    pub async fn open(workspace: &'ws Workspace, profile: &Profile) -> Result<Self> {
        workspace
            .init_target(profile)
            .await
            .context("init workspace target")?;

        let root = workspace.target.join(profile.as_str());
        let lock = root.join(".cargo-lock");
        let lock = LockFile::open(lock).await.context("open lockfile")?;
        let root = root
            .as_std_path()
            .relative_to(&workspace.target)
            .context("make root relative")?;

        Ok(Self {
            state: PhantomData,
            index: None,
            profile: profile.clone(),
            root,
            lock,
            workspace,
        })
    }

    /// Lock the directory.
    #[instrument(name = "ProfileDir::lock")]
    pub async fn lock(self) -> Result<ProfileDir<'ws, Locked>> {
        let lock = self.lock.lock().await.context("lock profile")?;
        let root = self.root.to_path(&self.workspace.target);
        let index = Index::recursive(&root)
            .await
            .map(Arc::new)
            .map(Some)
            .context("index target folder")?;
        Ok(ProfileDir {
            state: PhantomData,
            profile: self.profile,
            root: self.root,
            workspace: self.workspace,
            lock,
            index,
        })
    }
}

impl<'ws> ProfileDir<'ws, Locked> {
    /// Enumerate cache artifacts in the target directory for the dependency.
    ///
    /// For now in this context, a "cache artifact" is _any file_ inside the
    /// profile directory that is inside the `.fingerprint`, `build`, or `deps`
    /// directories, where the immediate subdirectory of that parent is prefixed
    /// by the name of the dependency.
    ///
    /// TODO: the above is probably overly broad for a cache; evaluate
    /// what filtering mechanism to apply to reduce invalidations and rework.
    #[instrument(name = "ProfileDir::enumerate_cache_artifacts")]
    pub async fn enumerate_cache_artifacts(
        &self,
        dependency: &Dependency,
    ) -> Result<Vec<Artifact>> {
        let index = self.index.as_ref().ok_or_eyre("files not indexed")?;

        // Fingerprint artifacts are straightforward:
        // if they're inside the `.fingerprint` directory,
        // and the subdirectory of `.fingerprint` starts with the name of
        // the dependency, then they should be backed up.
        //
        // Builds are the same as fingerprints, just with a different root:
        // instead of `.fingerprint`, they're looking for `build`.
        let standard = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .tuple_windows()
                    .next()
                    .is_some_and(|(parent, child)| {
                        child.as_str().starts_with(&dependency.name)
                            && (parent.as_str() == ".fingerprint" || parent.as_str() == "build")
                    })
            })
            .collect_vec();

        // Dependencies are totally different from the two above.
        // This directory is flat; inside we're looking for one primary file:
        // a `.d` file whose name starts with the name of the dependency.
        //
        // This file then lists other files (e.g. `*.rlib` and `*.rmeta`)
        // that this dependency built; these are _often_ (but not always)
        // named with a different prefix (often, but not always, "lib").
        //
        // Along the way we also grab any other random file in here that is
        // prefixed with the name of the dependency; so far this has been
        // `.rcgu.o` files which appear to be compiled codegen.
        //
        // Not all dependencies create `.d` files or indeed anything else
        // in the `deps` folder- from observation, it seems that this is the
        // case for purely proc-macro crates. This is honestly mostly ok,
        // because we want those to run anyway for now (until we figure out
        // a way to cache proc-macro invocations).
        let dependencies = index
            .files
            .iter()
            .filter(|(path, _)| {
                path.components()
                    .next()
                    .is_some_and(|part| part.as_str() == "deps")
            })
            .collect_vec();
        let dotd = dependencies.iter().find(|(path, _)| {
            path.components().nth(1).is_some_and(|part| {
                part.as_str().ends_with(".d") && part.as_str().starts_with(&dependency.name)
            })
        });
        let dependencies = if let Some((path, _)) = dotd {
            let outputs = Dotd::from_file(self, path)
                .await
                .context("parse .d file")?
                .outputs
                .into_iter()
                .collect::<HashSet<_>>();
            dependencies
                .into_iter()
                .filter(|(path, _)| {
                    outputs.contains(*path)
                        || path
                            .file_name()
                            .is_some_and(|name| name.starts_with(&dependency.name))
                })
                .collect_vec()
        } else {
            Vec::new()
        };

        // Now that we have our three sources of files,
        // we actually treat them all the same way!
        standard
            .into_iter()
            .chain(dependencies)
            .map(|(path, entry)| Artifact::builder().target(path).hash(&entry.hash).build())
            .inspect(|artifact| trace!(?artifact, "enumerated artifact"))
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// The root of the profile directory.
    pub fn root(&self) -> PathBuf {
        self.root.to_path(&self.workspace.target)
    }
}

/// Rust's compiler options for the current platform.
///
/// This isn't the _full_ set of options,
/// just what we need for caching.
//
// TODO: Support users cross compiling; probably need to parse argv?
// TODO: Determine minimum compiler version.
// TODO: Is there a better way to get this?
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize)]
pub struct RustcMetadata {
    /// The LLVM target triple.
    #[serde(rename = "llvm-target")]
    llvm_target: String,
}

impl RustcMetadata {
    /// Get platform metadata from the current compiler.
    #[instrument(name = "RustcMetadata::from_argv")]
    pub async fn from_argv(workspace_root: &Utf8Path, _argv: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new("rustc");

        // Bypasses the check that disallows using unstable commands on stable.
        cmd.env("RUSTC_BOOTSTRAP", "1");
        cmd.args(["-Z", "unstable-options", "--print", "target-spec-json"]);
        cmd.current_dir(workspace_root);
        let output = cmd.output().await.context("run rustc")?;
        if !output.status.success() {
            return Err(eyre!("invoke rustc"))
                .with_section(|| {
                    String::from_utf8_lossy(&output.stdout)
                        .to_string()
                        .header("Stdout:")
                })
                .with_section(|| {
                    String::from_utf8_lossy(&output.stderr)
                        .to_string()
                        .header("Stderr:")
                });
        }

        serde_json::from_slice::<RustcMetadata>(&output.stdout)
            .context("parse rustc output")
            .with_section(|| {
                String::from_utf8_lossy(&output.stdout)
                    .to_string()
                    .header("Rustc Output:")
            })
    }
}

/// A parsed Cargo .d file.
///
/// `.d` files are structured a little like makefiles, where each output
/// is on its own line followed by a colon followed by the inputs.
#[derive(Debug)]
pub struct Dotd {
    /// Recorded output paths, relative to the profile root.
    pub outputs: Vec<RelativePathBuf>,
}

impl Dotd {
    /// Construct an instance by parsing the file.
    #[instrument(name = "Dotd::from_file")]
    pub async fn from_file(
        profile: &ProfileDir<'_, Locked>,
        target: &RelativePath,
    ) -> Result<Self> {
        const DEP_EXTS: [&str; 3] = [".d", ".rlib", ".rmeta"];
        let profile_root = profile.root();
        let outputs = fs::read_buffered_utf8(target.to_path(&profile_root))
            .await
            .with_context(|| format!("read .d file: {target:?}"))?
            .ok_or_eyre("file does not exist")?
            .lines()
            .filter_map(|line| {
                let (output, _) = line.split_once(':')?;
                if DEP_EXTS.iter().any(|ext| output.ends_with(ext)) {
                    trace!(?line, ?output, "read .d line");
                    Utf8PathBuf::from_str(output)
                        .tap_err(|error| trace!(?line, ?output, ?error, "not a valid path"))
                        .ok()
                } else {
                    trace!(?line, "skipped .d line");
                    None
                }
            })
            .map(|output| -> Result<RelativePathBuf> {
                output
                    .strip_prefix(&profile_root)
                    .with_context(|| format!("make {output:?} relative to {profile_root:?}"))
                    .and_then(|p| RelativePathBuf::from_path(p).context("read path as utf8"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { outputs })
    }
}
