//! Benchmarks for caching cargo projects.

use color_eyre::{Result, eyre::Context};
use divan::Bencher;
use hurry::{
    cache::{FsCache, FsCas},
    cargo::{
        Dependency, Profile, Workspace, cache_target_from_workspace, restore_target_from_cache,
    },
    fs,
    hash::Blake3,
    mk_rel_dir,
    path::{AbsDirPath, JoinWith},
};
use location_macros::workspace_dir;
use tap::Pipe;
use tempfile::TempDir;

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 5)]
fn open() {
    let workspace = current_workspace();
    tokio::runtime::Runtime::new()
        .expect("set up tokio runtime")
        .block_on(async move {
            Workspace::from_argv_in_dir(&workspace, &[])
                .await
                .context("open workspace")
                .map(drop)
        })
        .expect("run benchmark");
}

#[divan::bench(sample_count = 5)]
fn index() {
    let workspace = current_workspace();
    tokio::runtime::Runtime::new()
        .expect("set up tokio runtime")
        .block_on(async move {
            let workspace = Workspace::from_argv_in_dir(&workspace, &[])
                .await
                .context("open workspace")?;
            workspace
                .open_profile_locked(&Profile::Debug)
                .await
                .context("open profile")
                .map(drop)
        })
        .expect("run benchmark");
}

#[divan::bench(sample_count = 5, skip_ext_time = true)]
fn backup(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let workspace = current_workspace();
            let (_tc, cache) = temporary_directory();
            let cas_root = cache.join(mk_rel_dir!("cas"));
            let cache_root = cache.join(mk_rel_dir!("ws"));
            tokio::runtime::Runtime::new()
                .expect("set up tokio runtime")
                .block_on(async move {
                    let cas = FsCas::open_dir(cas_root).await.context("open CAS")?;
                    let cache = FsCache::open_dir(cache_root)
                        .await
                        .context("open cache")?
                        .pipe(FsCache::lock)
                        .await
                        .context("lock cache")?;

                    // Since `ProfileDir` references workspace, we can't return
                    // it directly, but we want to avoid having to index and
                    // lock the profile folder during the benchmark.
                    // Given this, we just leak the workspace so that the
                    // reference is static.
                    let workspace = Workspace::from_argv_in_dir(&workspace, &[])
                        .await
                        .context("open workspace")?
                        .pipe(Box::new)
                        .pipe(Box::leak);
                    let target = workspace
                        .open_profile_locked(&Profile::Debug)
                        .await
                        .context("open profile")?;
                    Result::<_>::Ok((cas, cache, target))
                })
                .expect("set up benchmark")
        })
        .bench_values(|(cas, cache, target)| {
            tokio::runtime::Runtime::new()
                .expect("set up tokio runtime")
                .block_on(async move {
                    cache_target_from_workspace(&cas, &cache, &target, progress_noop)
                        .await
                        .context("backup target")
                })
                .expect("run benchmark")
        });
}

#[divan::bench(sample_count = 5, skip_ext_time = true)]
fn restore(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let local_workspace = current_workspace();
            let (_tw, temp_workspace) = temporary_directory();
            let (_tc, cache) = temporary_directory();
            let cas_root = cache.join(mk_rel_dir!("cas"));
            let cache_root = cache.join(mk_rel_dir!("ws"));
            tokio::runtime::Runtime::new()
                .expect("set up tokio runtime")
                .block_on(async move {
                    // We don't want to mess with the current workspace.
                    //
                    // Unfortunately this'll make the benchmark slower,
                    // but since we're running with `skip_ext_time`
                    // it should't be reported as benchmark time.
                    fs::copy_dir(&local_workspace, &temp_workspace)
                        .await
                        .context("copy current workspace to temp workspace")?;

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
                    }
                    fs::remove_dir_all(&workspace.target)
                        .await
                        .context("remove workspace target folder")?;

                    // Since `ProfileDir` references workspace, we can't return
                    // it directly, but we want to avoid having to index and
                    // lock the profile folder during the benchmark.
                    // Given this, we just leak the workspace so that the
                    // reference is static.
                    let workspace = Box::leak(Box::new(workspace));
                    let target = workspace
                        .open_profile_locked(&Profile::Debug)
                        .await
                        .context("open profile")?;
                    Result::<_>::Ok((cas, cache, target))
                })
                .expect("set up benchmark")
        })
        .bench_values(|(cas, cache, target)| {
            tokio::runtime::Runtime::new()
                .expect("set up tokio runtime")
                .block_on(async move {
                    restore_target_from_cache(&cas, &cache, &target, progress_noop)
                        .await
                        .context("restore target")
                })
                .expect("run benchmark")
        });
}

fn progress_noop(_key: &Blake3, _dep: &Dependency) {}

#[track_caller]
pub fn current_workspace() -> AbsDirPath {
    let ws = workspace_dir!();
    AbsDirPath::try_from(ws).unwrap_or_else(|err| panic!("parse {ws:?} as abs dir: {err:?}"))
}

#[track_caller]
fn temporary_directory() -> (TempDir, AbsDirPath) {
    let dir = TempDir::new().expect("create temporary directory");
    let path = AbsDirPath::try_from(dir.path()).expect("read temp dir as abs dir");
    (dir, path)
}
