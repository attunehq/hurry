use clap::Args;
use color_eyre::{
    Result,
    eyre::{Context, eyre},
};
use colored::Colorize;
use futures::TryStreamExt;
use hurry::{
    fs::{self, Metadata},
    path::{AbsFilePath, RelativeTo, SomeDirPath},
};
use itertools::Itertools;
use tracing::instrument;

/// Options for `debug metadata`
#[derive(Clone, Args, Debug)]
pub struct Options {
    /// The directory to inspect.
    path: SomeDirPath,
}

#[instrument]
pub async fn exec(options: Options) -> Result<()> {
    let root = options.path.try_as_abs_dir_using_cwd()?;

    // We have to buffer this so that we can sort the files; we want to sort the
    // files so that the output of two metadata commands can be diffed.
    let files = fs::walk_files(&root)
        .try_collect::<Vec<AbsFilePath>>()
        .await?;

    for path in files.into_iter().sorted() {
        let rel = path.relative_to(&root)?;
        let name = path
            .file_name()
            .ok_or_else(|| eyre!("file has no name: {path:?}"))?
            .to_string_lossy()
            .blue();

        let indent = "  ".repeat(rel.components().count().saturating_sub(1));
        let metadata = Metadata::from_file(&path).await.context("read metadata")?;
        println!("{indent}{name} -> {metadata:?}");
    }

    Ok(())
}
