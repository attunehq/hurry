use color_eyre::{Result, eyre::Context};
use hurry::{
    cache::{FsCache, FsCas},
    cargo::{
        Dependency, Profile, Workspace, cache_target_from_workspace, restore_target_from_cache,
    },
    fs,
    hash::Blake3,
    mk_rel_dir,
    path::{JoinWith, TryJoinWith},
};
use tap::Pipe;

use crate::{current_workspace, temporary_directory};

fn progress_noop(_key: &Blake3, _dep: &Dependency) {}

#[test_log::test(tokio::test)]
async fn open_workspace() -> Result<()> {
    let workspace = current_workspace();
    Workspace::from_argv_in_dir(&workspace, &[])
        .await
        .context("open workspace")
        .map(drop)
}

#[test_log::test(tokio::test)]
async fn open_index_workspace() -> Result<()> {
    let workspace = current_workspace();
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
    let workspace = current_workspace();
    let (_temp, tempdir) = temporary_directory();
    let cas_root = tempdir.join(mk_rel_dir!("cas"));
    let cache_root = tempdir.join(mk_rel_dir!("ws"));

    let cas = FsCas::open_dir(cas_root).await.context("open CAS")?;
    let cache = FsCache::open_dir(cache_root)
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
#[ignore = "Issue #17 blocks this from working now that we have proper typed paths"]
async fn restore_workspace() -> Result<()> {
    let local_workspace = current_workspace();
    let (_temp_cache, cache) = temporary_directory();
    let cas_root = cache.join(mk_rel_dir!("cas"));
    let cache_root = cache.join(mk_rel_dir!("ws"));
    let (_temp_ws, temp_workspace) = temporary_directory();

    // We don't want to mess with the current workspace.
    fs::copy_dir(&local_workspace, &temp_workspace)
        .await
        .context("copy current workspace to temp workspace")?;
    assert!(
        !fs::is_dir_empty(&temp_workspace).await?,
        "must have copied workspace"
    );

    let cas = FsCas::open_dir(cas_root).await.context("open CAS")?;
    let cache = FsCache::open_dir(cache_root)
        .await
        .context("open cache")?
        .pipe(FsCache::lock)
        .await
        .context("lock cache")?;

    let workspace = Workspace::from_argv_in_dir(&temp_workspace, &[])
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
    fs::remove_dir_all(&workspace.target)
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
        let subdir = target
            .root()
            .try_join_dir(name)
            .unwrap_or_else(|err| panic!("subdir {name:?} does not exist: {err:?}"));
        assert!(
            !fs::is_dir_empty(&subdir).await?,
            "{subdir:?} must have been restored",
        );
    }
    Ok(())
}
