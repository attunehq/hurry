use std::path::PathBuf;

use color_eyre::{Result, eyre::Context};
use hurry::{
    cache::{FsCache, FsCas},
    cargo::{
        Dependency, Profile, Workspace, cache_target_from_workspace, restore_target_from_cache,
    },
    fs,
    hash::Blake3,
};
use location_macros::workspace_dir;
use tap::Pipe;
use tempfile::TempDir;

fn progress_noop(_key: &Blake3, _dep: &Dependency) {}

#[test_log::test(tokio::test)]
async fn open_workspace() -> Result<()> {
    let workspace = PathBuf::from(workspace_dir!());
    Workspace::from_argv_in_dir(&workspace, &[])
        .await
        .context("open workspace")
        .map(drop)
}

#[test_log::test(tokio::test)]
async fn open_index_workspace() -> Result<()> {
    let workspace = PathBuf::from(workspace_dir!());
    let workspace = Workspace::from_argv_in_dir(&workspace, &[])
        .await
        .context("open workspace")?;
    workspace
        .open_profile_locked(&Profile::Debug)
        .await
        .context("open profile")
        .map(drop)
}

#[test_log::test(tokio::test)]
async fn backup_workspace() -> Result<()> {
    let workspace = PathBuf::from(workspace_dir!());
    let temp = TempDir::new().expect("create temporary directory");
    let cas_root = temp.path().join("cas");
    let cache_root = temp.path().join("ws");

    let cas = FsCas::open_dir_std(cas_root).await.context("open CAS")?;
    let cache = FsCache::open_dir_std(cache_root)
        .await
        .context("open cache")?
        .pipe(FsCache::lock)
        .await
        .context("lock cache")?;

    let workspace = Workspace::from_argv_in_dir(&workspace, &[])
        .await
        .context("open workspace")?;
    let target = workspace
        .open_profile_locked(&Profile::Debug)
        .await
        .context("open profile")?;

    cache_target_from_workspace(&cas, &cache, &target, progress_noop)
        .await
        .context("backup target")?;

    assert!(!cas.is_empty().await?, "cas must have files");
    assert!(!cache.is_empty().await?, "cas must have files");
    Ok(())
}

#[test_log::test(tokio::test)]
async fn restore_workspace() -> Result<()> {
    let local_workspace = PathBuf::from(workspace_dir!());
    let temp_workspace = TempDir::new().expect("create temporary directory");
    let temp = TempDir::new().expect("create temporary directory");
    let cas_root = temp.path().join("cas");
    let cache_root = temp.path().join("ws");

    // We don't want to mess with the current workspace.
    let workspace = temp_workspace.path();
    fs::copy_dir(local_workspace, workspace)
        .await
        .context("copy current workspace to temp workspace")?;
    assert!(
        !fs::is_dir_empty(workspace).await?,
        "must have copied workspace"
    );

    let cas = FsCas::open_dir_std(cas_root).await.context("open CAS")?;
    let cache = FsCache::open_dir_std(cache_root)
        .await
        .context("open cache")?
        .pipe(FsCache::lock)
        .await
        .context("lock cache")?;

    let workspace = Workspace::from_argv_in_dir(&workspace, &[])
        .await
        .context("open workspace")?;
    {
        let target = workspace
            .open_profile_locked(&Profile::Debug)
            .await
            .context("open profile")?;
        cache_target_from_workspace(&cas, &cache, &target, progress_noop)
            .await
            .context("backup target")?;
        assert!(!cas.is_empty().await?, "must have backed up files in CAS");
        assert!(!cache.is_empty().await?, "must have backed up files in CAS");
    }
    tokio::fs::remove_dir_all(&workspace.target)
        .await
        .context("remove workspace target folder")?;

    let target = workspace
        .open_profile_locked(&Profile::Debug)
        .await
        .context("open profile")?;
    restore_target_from_cache(&cas, &cache, &target, progress_noop)
        .await
        .context("restore target")?;

    assert!(!cas.is_empty().await?, "cas must have files");
    assert!(!cache.is_empty().await?, "cas must have files");

    // TODO: currently, we don't actually restore anything to `deps` or `build`
    // because we fail to parse the .d files since the project is moved.
    // This may need to be fixed as part of #17.
    for name in [/*"deps", "build",*/ ".fingerprint"] {
        let subdir = target.root().join(name);
        assert!(
            !fs::is_dir_empty(&subdir).await?,
            "{subdir:?} must have been restored",
        );
    }
    Ok(())
}
