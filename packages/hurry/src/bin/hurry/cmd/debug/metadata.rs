use async_walkdir::WalkDir;
use clap::Args;
use color_eyre::{Result, eyre::Context};
use colored::Colorize;
use futures::StreamExt;
use hurry::{
    fs::Metadata,
    path::{AbsSomePath, RelativeTo, SomeDirPath},
};
use tracing::instrument;

/// Options for `debug metadata`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// The directory to inspect.
    path: SomeDirPath,
}

const SPACE: &str = "  ";

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let root = options.path.try_as_abs_dir_using_cwd()?;
    let mut walker = WalkDir::new(root.as_std_path());
    while let Some(entry) = walker.next().await {
        let entry = entry.context("walk files")?;
        let path = AbsSomePath::try_from(entry.path())?;
        let rel = path.relative_to(&root)?;

        let name = entry.file_name();
        let name = name.to_string_lossy().blue();
        let indent = SPACE.repeat(rel.components().skip(1).count());

        let ft = entry.file_type().await.context("get file type")?;
        if ft.is_dir() {
            println!("{indent}{name}/");
        } else {
            let path = path.try_as_abs_file()?;
            let metadata = Metadata::from_file(&path).await.context("read metadata")?;
            println!("{indent}{name} -> {metadata:?}");
        }
    }

    Ok(())
}
