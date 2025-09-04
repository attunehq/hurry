use std::path::PathBuf;

use async_walkdir::WalkDir;
use clap::Args;
use color_eyre::{Result, eyre::Context};
use colored::Colorize;
use futures::StreamExt;
use hurry::fs::Metadata;
use relative_path::PathExt;
use tracing::instrument;

/// Options for `debug cargo metadata`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// The directory to inspect.
    path: PathBuf,
}

const SPACE: &str = "  ";

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let mut walker = WalkDir::new(&options.path);
    while let Some(entry) = walker.next().await {
        let entry = entry.context("walk files")?;
        let path = entry
            .path()
            .relative_to(&options.path)
            .context("make relative path")?;

        let name = entry.file_name();
        let name = name.to_string_lossy().blue();
        let indent = SPACE.repeat(path.components().skip(1).count());

        let ft = entry.file_type().await.context("get file type")?;
        if ft.is_dir() {
            println!("{indent}{name}/");
        } else {
            let metadata = Metadata::from_file(entry.path())
                .await
                .context("read metadata")?;
            println!("{indent}{name} -> {metadata:?}");
        }
    }

    Ok(())
}
