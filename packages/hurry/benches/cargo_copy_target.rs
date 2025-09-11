//! Benchmarks for copying `target/` directories of Cargo projects.
//!
//! Note: these benchmarks use the `target/` of the _current_ project;
//! as such the benchmark changing doesn't _automatically_ mean that
//! performance actually changed as the `target/` folder may have also changed.

use color_eyre::Result;
use hurry::{
    mk_rel_dir,
    path::{AbsDirPath, JoinWith},
};
use location_macros::workspace_dir;
use tempfile::TempDir;

fn main() {
    divan::main();
}

mod baseline {
    use super::*;

    #[divan::bench(sample_count = 5)]
    fn cp() {
        let target = current_target();
        let (_temp, destination) = temporary_directory();

        std::process::Command::new("cp")
            .arg("-r")
            .arg(target.as_os_str())
            .arg(destination.as_os_str())
            .output()
            .expect("copy with cp");
    }

    #[cfg(target_os = "macos")]
    #[divan::bench(sample_count = 5)]
    fn cp_cow() {
        let target = current_target();
        let (_temp, destination) = temporary_directory();

        std::process::Command::new("cp")
            .arg("-c")
            .arg("-r")
            .arg(target.as_os_str())
            .arg(destination.as_os_str())
            .output()
            .expect("copy with cp");
    }
}

mod sync {
    use super::*;

    mod single_threaded {
        use std::{collections::HashSet, path::Path, usize};

        use itertools::Itertools;

        use super::*;

        #[divan::bench(sample_count = 5)]
        fn walkdir_single_pass() {
            let target = current_target();
            let (temp, _) = temporary_directory();

            for entry in walkdir::WalkDir::new(target.as_std_path()) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                let rel = entry
                    .path()
                    .strip_prefix(target.as_std_path())
                    .expect("make relative");
                let src = entry.path();
                let dst = temp.path().join(rel);

                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent).expect("create parents");
                }
                std::fs::copy(src, &dst)
                    .unwrap_or_else(|err| panic!("copy {src:?} to {dst:?}: {err}"));
            }
        }

        #[divan::bench(sample_count = 5)]
        fn walkdir_two_pass() {
            let target = current_target();
            let (temp, _) = temporary_directory();

            let mut index = HashSet::new();
            for entry in walkdir::WalkDir::new(target.as_std_path()) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                let rel = entry
                    .path()
                    .strip_prefix(target.as_std_path())
                    .expect("make relative");
                index.insert(rel.to_path_buf());
            }

            let parents = index
                .iter()
                .filter_map(|p| p.parent())
                .sorted_by_cached_key(|p| usize::MAX - p.ancestors().count())
                .fold(Vec::<&Path>::new(), |mut kept, p| {
                    if !kept.iter().any(|k| k.starts_with(&p)) {
                        kept.push(p);
                    }
                    kept
                });
            for parent in parents {
                let target = temp.path().join(parent);
                std::fs::create_dir_all(&target)
                    .unwrap_or_else(|err| panic!("create parent {target:?}: {err}"));
            }
            for file in index {
                let src = target.as_std_path().join(&file);
                let dst = temp.path().join(file);
                std::fs::copy(&src, &dst)
                    .unwrap_or_else(|err| panic!("copy {src:?} to {dst:?}: {err}"));
            }
        }
    }

    mod using_rayon {
        use std::{collections::HashSet, path::Path, usize};

        use color_eyre::eyre::Context;
        use itertools::Itertools;
        use rayon::iter::{IntoParallelIterator, ParallelBridge, ParallelIterator};

        use super::*;

        #[divan::bench(sample_count = 5)]
        fn walkdir_single_pass() {
            let target = current_target();
            let (temp, _) = temporary_directory();

            walkdir::WalkDir::new(target.as_std_path())
                .into_iter()
                .par_bridge()
                .try_for_each(|entry| -> Result<()> {
                    let entry = entry.context("walk files")?;
                    if !entry.file_type().is_file() {
                        return Ok(());
                    }

                    let rel = entry
                        .path()
                        .strip_prefix(target.as_std_path())
                        .context("make relative")?;
                    let src = entry.path();
                    let dst = temp.path().join(rel);

                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent).context("create parents")?;
                    }
                    std::fs::copy(src, &dst)
                        .with_context(|| format!("copy {src:?} to {dst:?}"))
                        .map(drop)
                })
                .expect("copy files");
        }

        #[divan::bench(sample_count = 5)]
        fn walkdir_two_pass() {
            let target = current_target();
            let (temp, _) = temporary_directory();

            let mut index = HashSet::new();
            for entry in walkdir::WalkDir::new(target.as_std_path()) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                let rel = entry
                    .path()
                    .strip_prefix(target.as_std_path())
                    .expect("make relative");
                index.insert(rel.to_path_buf());
            }

            index
                .iter()
                .filter_map(|p| p.parent())
                .sorted_by_cached_key(|p| usize::MAX - p.ancestors().count())
                .fold(Vec::<&Path>::new(), |mut kept, p| {
                    if !kept.iter().any(|k| k.starts_with(&p)) {
                        kept.push(p);
                    }
                    kept
                })
                .into_par_iter()
                .try_for_each(|parent| -> Result<()> {
                    let target = temp.path().join(parent);
                    std::fs::create_dir_all(&target)
                        .with_context(|| format!("create parent {target:?}"))
                })
                .expect("create parents");

            index
                .into_par_iter()
                .try_for_each(|file| -> Result<()> {
                    let src = target.as_std_path().join(&file);
                    let dst = temp.path().join(file);
                    std::fs::copy(&src, &dst)
                        .with_context(|| format!("copy {src:?} to {dst:?}"))
                        .map(drop)
                })
                .expect("copy files");
        }

        #[divan::bench(sample_count = 5)]
        fn jwalk_single_pass() {
            let target = current_target();
            let (temp, _) = temporary_directory();

            jwalk::WalkDir::new(target.as_std_path())
                .into_iter()
                .par_bridge()
                .try_for_each(|entry| -> Result<()> {
                    let entry = entry.context("walk files")?;
                    if !entry.file_type().is_file() {
                        return Ok(());
                    }

                    let src = entry.path();
                    let rel = src
                        .strip_prefix(target.as_std_path())
                        .context("make relative")?;
                    let dst = temp.path().join(rel);

                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent).context("create parents")?;
                    }
                    std::fs::copy(&src, &dst)
                        .with_context(|| format!("copy {src:?} to {dst:?}"))
                        .map(drop)
                })
                .expect("copy files");
        }
    }
}

mod using_tokio {
    use color_eyre::eyre::{Context, eyre};
    use futures::{StreamExt, TryStreamExt};

    use super::*;

    #[divan::bench(sample_count = 5)]
    fn naive() {
        let target = current_target();
        let (temp, _) = temporary_directory();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            let mut walker = async_walkdir::WalkDir::new(target.as_std_path());
            while let Some(entry) = walker.next().await {
                let entry = entry.context("walk files")?;
                let ft = entry.file_type().await.context("get type")?;
                if !ft.is_file() {
                    continue;
                }

                let src = entry.path();
                let rel = src
                    .strip_prefix(target.as_std_path())
                    .context("make relative")?;
                let dst = temp.path().join(rel);

                if let Some(parent) = dst.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .context("create parents")?;
                }
                tokio::fs::copy(&src, &dst)
                    .await
                    .with_context(|| format!("copy {src:?} to {dst:?}"))?;
            }

            Ok(())
        });
        copy.expect("copy files");
    }

    #[divan::bench(sample_count = 5, args = [1, 10, 100, 1000])]
    fn concurrent(concurrency: usize) {
        let target = current_target();
        let (temp, _) = temporary_directory();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            async_walkdir::WalkDir::new(target.as_std_path())
                .map_err(|err| eyre!(err))
                .try_for_each_concurrent(Some(concurrency), |entry| {
                    let target = target.clone();
                    let temp = temp.path().to_path_buf();
                    async move {
                        let ft = entry.file_type().await.context("get type")?;
                        if !ft.is_file() {
                            return Ok(());
                        }

                        let src = entry.path();
                        let rel = src
                            .strip_prefix(target.as_std_path())
                            .context("make relative")?;
                        let dst = temp.join(rel);

                        if let Some(parent) = dst.parent() {
                            tokio::fs::create_dir_all(parent)
                                .await
                                .context("create parents")?;
                        }
                        tokio::fs::copy(&src, &dst)
                            .await
                            .with_context(|| format!("copy {src:?} to {dst:?}"))
                            .map(drop)
                    }
                })
                .await
        });
        copy.expect("copy files");
    }

    mod hurry_fs {
        use hurry::path::{JoinWith, RelativeTo};

        use super::*;

        #[divan::bench(sample_count = 5)]
        fn naive() {
            let target = current_target();
            let (_temp, tempdir) = temporary_directory();
            let runtime = tokio::runtime::Runtime::new().expect("create runtime");

            let copy: Result<()> = runtime.block_on(async move {
                let mut walker = hurry::fs::walk_files(&target);
                while let Some(entry) = walker.next().await {
                    let src = entry.context("walk files")?;

                    let rel = src.relative_to(&target).context("make relative")?;
                    let dst = tempdir.join(rel);

                    hurry::fs::copy_file(&src, &dst)
                        .await
                        .with_context(|| format!("copy {src:?} to {dst:?}"))?;
                }

                Ok(())
            });
            copy.expect("copy files");
        }

        #[divan::bench(sample_count = 5, args = [1, 10, 100, 1000])]
        fn concurrent(concurrency: usize) {
            let target = current_target();
            let (_temp, tempdir) = temporary_directory();
            let runtime = tokio::runtime::Runtime::new().expect("create runtime");

            let copy: Result<()> = runtime.block_on(async move {
                hurry::fs::copy_dir_with_concurrency(concurrency, &target, &tempdir)
                    .await
                    .map(drop)
            });
            copy.expect("copy files");
        }
    }
}

#[track_caller]
pub fn current_workspace() -> AbsDirPath {
    let ws = workspace_dir!();
    AbsDirPath::try_from(ws).unwrap_or_else(|err| panic!("parse {ws:?} as abs dir: {err:?}"))
}

#[track_caller]
fn current_target() -> AbsDirPath {
    current_workspace().join(mk_rel_dir!("target"))
}

#[track_caller]
fn temporary_directory() -> (TempDir, AbsDirPath) {
    let dir = TempDir::new().expect("create temporary directory");
    let path = AbsDirPath::try_from(dir.path()).expect("read temp dir as abs dir");
    (dir, path)
}
