use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use color_eyre::{Result, eyre::Context};
use futures::{StreamExt, TryStreamExt};
use hurry::fs::{self, Metadata};
use location_macros::workspace_dir;
use relative_path::{PathExt, RelativePathBuf};
use tempfile::TempDir;

#[tokio::test]
async fn copy_files_diff() -> Result<()> {
    let _ = color_eyre::install()?;

    let target = PathBuf::from(workspace_dir!()).join("target");
    let temp = TempDir::new().context("create temporary directory")?;
    fs::copy_dir(&target, temp.path())
        .await
        .context("copy folder")?;

    let (source, destination) = tokio::try_join!(
        DirectoryMetadata::from_directory(&target),
        DirectoryMetadata::from_directory(temp.path())
    )
    .context("diff directories")?;

    pretty_assertions::assert_eq!(source, destination, "directories should be equivalent");

    Ok(())
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct DirectoryMetadata(BTreeMap<RelativePathBuf, Metadata>);

impl DirectoryMetadata {
    async fn from_directory(root: impl AsRef<Path>) -> Result<DirectoryMetadata> {
        let root = root.as_ref();
        fs::walk_files(root)
            .map(|entry| async move {
                let entry = entry.context("walk directory")?;
                let path = entry.path();
                let metadata = Metadata::from_file(&path).await.context("get metadata")?;
                let path = path.relative_to(&root).context("make relative")?;
                Ok((path, metadata))
            })
            .buffer_unordered(fs::DEFAULT_CONCURRENCY)
            .try_filter_map(|(path, meta)| async move {
                match meta {
                    Some(meta) => Ok(Some((path, meta))),
                    None => Ok(None),
                }
            })
            .try_collect::<BTreeMap<_, _>>()
            .await
            .map(Self)
    }
}
