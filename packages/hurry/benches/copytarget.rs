use std::path::PathBuf;

use color_eyre::Result;
use location_macros::workspace_dir;
use tempfile::TempDir;
use xshell::{Shell, cmd};

fn main() {
    divan::main();
}

mod baseline {
    use super::*;

    #[divan::bench(sample_count = 1)]
    fn cp() {
        let (target, temp) = setup();
        let destination = temp.path();

        let sh = Shell::new().expect("create xshell");
        cmd!(sh, "cp -r {target} {destination}")
            .output()
            .expect("copy with cp");
    }

    #[cfg(target_os = "macos")]
    #[divan::bench(sample_count = 1)]
    fn cp_cow() {
        let (target, temp) = setup();
        let destination = temp.path();

        let sh = Shell::new().expect("create xshell");
        cmd!(sh, "cp -c -r {target} {destination}")
            .output()
            .expect("copy with cp");
    }

    #[cfg(target_os = "linux")]
    #[divan::bench(sample_count = 1)]
    fn cp_reflink() {
        let (target, temp) = setup();
        let destination = temp.path();

        let sh = Shell::new().expect("create xshell");
        cmd!(sh, "cp --reflink -r {target} {destination}")
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

        #[divan::bench(sample_count = 1)]
        fn walkdir_single_pass() {
            let (target, temp) = setup();

            for entry in walkdir::WalkDir::new(&target) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                let rel = entry.path().strip_prefix(&target).expect("make relative");
                let src = entry.path();
                let dst = temp.path().join(rel);

                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent).expect("create parents");
                }
                std::fs::copy(src, &dst)
                    .unwrap_or_else(|err| panic!("copy {src:?} to {dst:?}: {err}"));
            }
        }

        #[divan::bench(sample_count = 1)]
        fn walkdir_two_pass() {
            let (target, temp) = setup();

            let mut index = HashSet::new();
            for entry in walkdir::WalkDir::new(&target) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                let rel = entry.path().strip_prefix(&target).expect("make relative");
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
                let src = target.join(&file);
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

        #[divan::bench(sample_count = 1)]
        fn walkdir_single_pass() {
            let (target, temp) = setup();

            walkdir::WalkDir::new(&target)
                .into_iter()
                .par_bridge()
                .try_for_each(|entry| -> Result<()> {
                    let entry = entry.context("walk files")?;
                    if !entry.file_type().is_file() {
                        return Ok(());
                    }

                    let rel = entry
                        .path()
                        .strip_prefix(&target)
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

        #[divan::bench(sample_count = 1)]
        fn walkdir_two_pass() {
            let (target, temp) = setup();

            let mut index = HashSet::new();
            for entry in walkdir::WalkDir::new(&target) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                let rel = entry.path().strip_prefix(&target).expect("make relative");
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
                    let src = target.join(&file);
                    let dst = temp.path().join(file);
                    std::fs::copy(&src, &dst)
                        .with_context(|| format!("copy {src:?} to {dst:?}"))
                        .map(drop)
                })
                .expect("copy files");
        }

        #[divan::bench(sample_count = 1)]
        fn jwalk_single_pass() {
            let (target, temp) = setup();

            jwalk::WalkDir::new(&target)
                .into_iter()
                .par_bridge()
                .try_for_each(|entry| -> Result<()> {
                    let entry = entry.context("walk files")?;
                    if !entry.file_type().is_file() {
                        return Ok(());
                    }

                    let src = entry.path();
                    let rel = src.strip_prefix(&target).context("make relative")?;
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

    #[divan::bench(sample_count = 1)]
    fn naive() {
        let (target, temp) = setup();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            let mut walker = async_walkdir::WalkDir::new(&target);
            while let Some(entry) = walker.next().await {
                let entry = entry.context("walk files")?;
                let ft = entry.file_type().await.context("get type")?;
                if !ft.is_file() {
                    continue;
                }

                let src = entry.path();
                let rel = src.strip_prefix(&target).context("make relative")?;
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

    #[divan::bench(sample_count = 1, args = [1, 10, 100, 1000])]
    fn concurrent(concurrency: usize) {
        let (target, temp) = setup();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            async_walkdir::WalkDir::new(&target)
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
                        let rel = src.strip_prefix(&target).context("make relative")?;
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
}

mod hurry_fs {
    use std::{path::Path, time::SystemTime};

    use color_eyre::eyre::{Context, eyre};
    use filetime::FileTime;
    use futures::{StreamExt, TryStreamExt};
    use tap::TapFallible;
    use tokio::{fs::File, task::spawn_blocking};
    use tracing::{instrument, trace};

    use super::*;

    #[divan::bench(sample_count = 1)]
    fn naive() {
        let (target, temp) = setup();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            let mut walker = async_walkdir::WalkDir::new(&target);
            while let Some(entry) = walker.next().await {
                let entry = entry.context("walk files")?;
                let ft = entry.file_type().await.context("get type")?;
                if !ft.is_file() {
                    continue;
                }

                let src = entry.path();
                let rel = src.strip_prefix(&target).context("make relative")?;
                let dst = temp.path().join(rel);

                hurry::fs::copy_file(&src, &dst)
                    .await
                    .with_context(|| format!("copy {src:?} to {dst:?}"))?;
            }

            Ok(())
        });
        copy.expect("copy files");
    }

    #[divan::bench(sample_count = 1, args = [1, 10, 100, 1000])]
    fn concurrent(concurrency: usize) {
        let (target, temp) = setup();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            async_walkdir::WalkDir::new(&target)
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
                        let rel = src.strip_prefix(&target).context("make relative")?;
                        let dst = temp.join(rel);

                        hurry::fs::copy_file(&src, &dst)
                            .await
                            .with_context(|| format!("copy {src:?} to {dst:?}"))
                            .map(drop)
                    }
                })
                .await
        });
        copy.expect("copy files");
    }
}

#[track_caller]
fn setup() -> (PathBuf, TempDir) {
    let target = PathBuf::from(workspace_dir!()).join("target");
    let temp = TempDir::new().expect("create temporary directory");
    (target, temp)
}
