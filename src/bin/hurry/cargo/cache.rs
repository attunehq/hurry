use std::{
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

#[cfg(target_family = "unix")]
use std::os::unix;
#[cfg(target_family = "windows")]
use std::os::windows;

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use homedir::my_home;
use include_dir::Dir;
use rusqlite::Connection;
use rusqlite_migration::Migrations;
use tracing::{debug, instrument, trace};

#[derive(Debug)]
pub struct WorkspaceCache {
    pub workspace_cache_path: PathBuf,
    pub cas_path: PathBuf,
    pub metadb: Connection,
}

impl WorkspaceCache {
    #[instrument]
    pub fn new<P>(workspace_path: P) -> Result<Self>
    where
        P: AsRef<Path>,
        P: std::fmt::Debug,
    {
        // Check whether the user cache exists, and create it if it
        // doesn't.
        let user_cache_path = &USER_CACHE_PATH;
        trace!(?user_cache_path, "checking user cache");
        if !fs::exists(&**user_cache_path).context("could not read user hurry cache")? {
            fs::create_dir_all(&**user_cache_path).context("could not create user hurry cache")?;
        }

        // Check whether the CAS exists, and create it if it doesn't.
        let cas_path = user_cache_path.join("cas");
        trace!(?cas_path, "checking CAS");
        if !fs::exists(&cas_path).context("could not read CAS")? {
            fs::create_dir_all(&cas_path).context("could not create CAS")?;
        }

        // Check whether the workspace cache exists, and create it if it
        // doesn't.
        let workspace_cache_path = {
            let mut path = user_cache_path.join("workspaces");
            path.push(
                blake3::hash(workspace_path.as_ref().as_os_str().as_encoded_bytes())
                    .to_hex()
                    .as_str(),
            );
            path
        };
        trace!(?workspace_cache_path, "checking workspace cache");
        if !fs::exists(&workspace_cache_path).context("could not read workspace hurry cache")? {
            fs::create_dir_all(&workspace_cache_path)
                .context("could not create workspace hurry cache")?;
        }

        // Check whether the workspace target cache exists, and create it if it
        // doesn't.
        let target_cache_path = workspace_cache_path.join("target");
        trace!(?target_cache_path, "checking workspace target cache");
        if !fs::exists(&target_cache_path).context("could not read workspace target cache")? {
            fs::create_dir_all(&target_cache_path)
                .context("could not create workspace target cache")?;
        }

        // Check whether the workspace target/ is correctly linked to the
        // workspace target cache, and create a symlink if it is not.
        //
        // NOTE: We call `fs::symlink_metadata` and match on explicit error
        // cases because `fs::exists` returns `Ok(false)` for broken symlinks
        // and so cannot distinguish between "there is no file" and "there is a
        // file, but it's a broken symlink", which we need to handle
        // differently.
        let target_path = workspace_path.as_ref().join("target");
        trace!(?target_path, "checking workspace target/");
        ensure_symlink(&target_cache_path, &target_path)
            .context("could not symlink workspace target/ to cache")?;

        // Open the workspace metadata database and migrate it if necessary.
        let mut metadb = Connection::open(workspace_cache_path.join("meta.db"))
            .context("could not read workspace cache state")?;
        trace!(pending_migrations = ?MIGRATIONS.pending_migrations(&mut metadb), "checking migrations");
        MIGRATIONS
            .to_latest(&mut metadb)
            .context("could not migrate workspace cache state")?;

        Ok(Self {
            metadb,
            workspace_cache_path,
            cas_path,
        })
    }
}

static USER_CACHE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = my_home().unwrap().unwrap();
    path.push(".cache");
    path.push("hurry");
    path.push("v1");
    path.push("cargo");
    path
});

#[instrument]
fn ensure_symlink(original: &PathBuf, link: &PathBuf) -> Result<()> {
    // NOTE: We call `fs::symlink_metadata` and match on explicit error
    // cases because `fs::exists` returns `Ok(false)` for broken symlinks
    // and so cannot distinguish between "there is no file" and "there is a
    // file, but it's a broken symlink", which we need to handle
    // differently.
    let cache_metadata = fs::symlink_metadata(&link);
    match cache_metadata {
        Ok(metadata) => {
            trace!(?metadata, "link metadata");
            if metadata.is_symlink() {
                let target_symlink_path = fs::read_link(&link).context("could not read symlink")?;
                trace!(?target_symlink_path, "symlink target");
                if target_symlink_path == *original {
                    return Ok(());
                } else {
                    fs::remove_file(&link).context("could not remove stale symlink")?;
                }
            } else if metadata.is_file() {
                fs::remove_file(&link).context("could not remove file")?;
            } else if metadata.is_dir() {
                // TODO: If there already is a `target/` folder, should we index
                // its contents? This might not be sound, since we don't have a
                // guarantee that the current `src/` are the files that
                // generated the artifacts in `target/`. (We normally have this
                // guarantee because we are wrapping an invocation of `cargo
                // build`).
                fs::remove_dir_all(&original).context("could not overwrite link target")?;
                fs::rename(&link, &original).context("could not move link to target")?;
            } else {
                bail!("file has unknown file type: {:?}", metadata.file_type());
            }
        }
        Err(e) => {
            debug!(read_error = ?e, "could not read file");
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e).context("could not check whether file exists");
            }
        }
    }

    // If we're here, the workspace target/ cache does not exist, so we can create
    // a symlink.
    symlink_dir(&original, &link).context("could not create symlink")?;

    Ok(())
}

#[cfg(target_family = "windows")]
fn symlink_dir(original: &PathBuf, link: &PathBuf) -> Result<()> {
    windows::fs::symlink_dir(original, link)?;
    Ok(())
}

#[cfg(target_family = "unix")]
fn symlink_dir(original: &PathBuf, link: &PathBuf) -> Result<()> {
    unix::fs::symlink(original, link)?;
    Ok(())
}

static MIGRATIONS_DIR: Dir =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/bin/hurry/cargo/migrations");

static MIGRATIONS: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::from_directory(&MIGRATIONS_DIR).unwrap());
