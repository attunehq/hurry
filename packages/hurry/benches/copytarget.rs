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

                copy_file(&src, &dst)
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

                        copy_file(&src, &dst)
                            .await
                            .with_context(|| format!("copy {src:?} to {dst:?}"))
                            .map(drop)
                    }
                })
                .await
        });
        copy.expect("copy files");
    }

    /// Copy the file from `src` to `dst` preserving metadata.
    ///
    /// We can't actually reference the implementation in `hurry::fs`
    /// as it's in a bin crate; this (and other functions it calls) is a copy.
    #[instrument]
    async fn copy_file(
        src: impl AsRef<Path> + std::fmt::Debug,
        dst: impl AsRef<Path> + std::fmt::Debug,
    ) -> Result<()> {
        // Manually opening the source file allows us to access the stat info directly,
        // without an additional syscall to stat directly.
        let mut src = tokio::fs::File::open(src)
            .await
            .context("open source file")?;
        let src_meta = src.metadata().await.context("get source metadata")?;

        // If we can't read the actual times from the stat, default to unix epoch
        // so that we don't break the build system.
        //
        // We could promote this to an actual error, but since the rust compiler is ultimately
        // what's going to read this, this is simpler: it'll just transparently rebuild anything
        // that we had to set like this (since the source file will obviously be newer).
        //
        // In other words, this forms a safe "fail closed" system since
        // the rust compiler is the ultimate authority here.
        let src_mtime = src_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let src_atime = src_meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH);
        if let Some(parent) = dst.as_ref().parent() {
            create_dir_all(parent)
                .await
                .context("create parent directory")?;
        }

        // Manually opening the destination file allows us to set the metadata directly,
        // without the additional syscall to touch the file metadata.
        //
        // We don't currently care about any other metadata (e.g. permission bits, read only, etc)
        // since the rust compiler is the ultimate arbiter of this data and will reject/rebuild
        // anything that is out of sync.
        //
        // If we find that we have excessive rebuilds we can revisit this.
        let mut dst = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(dst)
            .await
            .context("open destination file")?;
        let bytes = tokio::io::copy(&mut src, &mut dst)
            .await
            .context("copy file contents")?;

        // Using the `filetime` crate here instead of the stdlib because it's cross platform.
        let mtime = FileTime::from_system_time(src_mtime);
        let atime = FileTime::from_system_time(src_atime);
        trace!(?src, ?dst, ?mtime, ?atime, ?bytes, "copy file");

        // We need to get the raw handle for filetime operations
        let dst = set_file_handle_times(dst, Some(atime), Some(mtime))
            .await
            .context("set destination file times")?;

        // And finally, we have to sync the file to disk so that we are sure it's actually finished copying
        // before we move on. Technically we could leave this up to the FS, but this is safer.
        dst.sync_all().await.context("sync destination file")
    }

    #[instrument]
    async fn create_dir_all(dir: impl AsRef<Path> + std::fmt::Debug) -> Result<()> {
        let dir = dir.as_ref();
        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("create dir: {dir:?}"))
            .tap_ok(|_| trace!(?dir, "create directory"))
    }

    /// Update the `atime` and `mtime` of a file handle.
    /// Returns the same file handle after the update.
    #[instrument]
    pub async fn set_file_handle_times(
        file: File,
        atime: Option<FileTime>,
        mtime: Option<FileTime>,
    ) -> Result<File> {
        match (mtime, atime) {
            (None, None) => Ok(file),
            (mtime, atime) => {
                let file = file.into_std().await;
                spawn_blocking(move || {
                    filetime::set_file_handle_times(&file, atime, mtime).map(|_| file)
                })
                .await
                .context("join thread")?
                .context("update handle")
                .map(File::from_std)
            }
        }
    }
}

#[track_caller]
fn setup() -> (PathBuf, TempDir) {
    let target = PathBuf::from(workspace_dir!()).join("target");
    let temp = TempDir::new().expect("create temporary directory");
    (target, temp)
}
