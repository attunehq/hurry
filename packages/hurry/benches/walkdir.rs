//! Benchmarks for walking directories of Cargo projects.
//!
//! Note: these benchmarks use the `target/` of the _current_ project;
//! as such the benchmark changing doesn't _automatically_ mean that
//! performance actually changed as the `target/` folder may have also changed.

#![allow(
    clippy::disallowed_methods,
    reason = "Permit sync std::fs methods in benchmarks"
)]

use color_eyre::Result;
use hurry::{
    mk_rel_dir,
    path::{AbsDirPath, JoinWith},
};
use location_macros::workspace_dir;

fn main() {
    divan::main();
}

mod sync {
    use super::*;

    mod single_threaded {
        use std::hint::black_box;

        use super::*;

        #[divan::bench(sample_count = 5)]
        fn walkdir() {
            let target = current_target();

            for entry in walkdir::WalkDir::new(target.as_std_path()) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                black_box(entry);
            }
        }

        #[divan::bench(sample_count = 5)]
        fn jwalk() {
            let target = current_target();

            for entry in jwalk::WalkDir::new(target.as_std_path()) {
                let entry = entry.expect("walk files");
                if !entry.file_type().is_file() {
                    continue;
                }

                black_box(entry);
            }
        }
    }

    mod multithread {
        use std::hint::black_box;

        use color_eyre::eyre::Context;
        use rayon::iter::{ParallelBridge, ParallelIterator};

        use super::*;

        #[divan::bench(sample_count = 5)]
        fn walkdir_rayon() {
            let target = current_target();

            walkdir::WalkDir::new(target.as_std_path())
                .into_iter()
                .par_bridge()
                .try_for_each(|entry| -> Result<()> {
                    let entry = entry.context("walk files")?;
                    if !entry.file_type().is_file() {
                        return Ok(());
                    }

                    black_box(entry);
                    Ok(())
                })
                .expect("walk files");
        }

        #[divan::bench(sample_count = 5)]
        fn jwalk_rayon() {
            let target = current_target();

            jwalk::WalkDir::new(target.as_std_path())
                .into_iter()
                .par_bridge()
                .try_for_each(|entry| -> Result<()> {
                    let entry = entry.context("walk files")?;
                    if !entry.file_type().is_file() {
                        return Ok(());
                    }

                    black_box(entry);
                    Ok(())
                })
                .expect("copy files");
        }
    }
}

mod using_tokio {
    use std::hint::black_box;

    use color_eyre::eyre::{Context, eyre};
    use futures::{StreamExt, TryStreamExt};

    use super::*;

    #[divan::bench(sample_count = 5)]
    fn async_walkdir() {
        let target = current_target();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            let mut walker = async_walkdir::WalkDir::new(target.as_std_path());
            while let Some(entry) = walker.next().await {
                let entry = entry.context("walk files")?;
                let ft = entry.file_type().await.context("get type")?;
                if !ft.is_file() {
                    continue;
                }

                black_box(entry);
            }

            Ok(())
        });
        copy.expect("copy files");
    }

    #[divan::bench(sample_count = 5, args = [1, 10, 100, 1000])]
    fn concurrent(concurrency: usize) {
        let target = current_target();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let copy: Result<()> = runtime.block_on(async move {
            async_walkdir::WalkDir::new(target.as_std_path())
                .map_err(|err| eyre!(err))
                .try_for_each_concurrent(Some(concurrency), |entry| async move {
                    let ft = entry.file_type().await.context("get type")?;
                    if !ft.is_file() {
                        return Ok(());
                    }

                    black_box(entry);
                    Ok(())
                })
                .await
        });
        copy.expect("copy files");
    }

    mod hurry_fs {
        use super::*;

        #[divan::bench(sample_count = 5)]
        fn walk_files() {
            let target = current_target();
            let runtime = tokio::runtime::Runtime::new().expect("create runtime");

            let copy: Result<()> = runtime.block_on(async move {
                let mut walker = hurry::fs::walk_files(&target);
                while let Some(entry) = walker.next().await {
                    let entry = entry.context("walk files")?;
                    black_box(entry);
                }

                Ok(())
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
