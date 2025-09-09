use std::collections::BTreeMap;

use color_eyre::{Result, eyre::Context};
use futures::{StreamExt, TryStreamExt};
use hurry::{
    fs::{self, Metadata},
    path::{AbsDirPath, RelFilePath, RelativeTo},
};
use pretty_assertions::assert_eq;

use crate::{current_target, temporary_directory};

#[test_log::test(tokio::test)]
async fn copy_files_diff() -> Result<()> {
    // Concurrent tests might mess with files in the current project's target
    // directory (notably: lockfiles).
    //
    // We're pretty confident that `cp -r` works; as such we use it to copy
    // the target to a tempdir and then test against that.
    let workspace = current_target();
    let (_st, src) = temporary_directory();
    let (_dt, dst) = temporary_directory();

    tokio::process::Command::new("cp")
        .arg("-r")
        .arg(workspace.as_os_str())
        .arg(src.as_os_str())
        .output()
        .await
        .with_context(|| format!("copy {workspace:?} to {src:?} using 'cp'"))?;

    // Now we copy using our native functionality from the copy to _another_
    // copy; this way we can test that our copy works as expected without
    // having racing tests.
    fs::copy_dir(&src, &dst)
        .await
        .with_context(|| format!("copy {src:?} to {dst:?} natively"))?;
    let (source, destination) = tokio::try_join!(
        DirectoryMetadata::from_directory(&src),
        DirectoryMetadata::from_directory(&dst)
    )
    .with_context(|| format!("diff {src:?} and {dst:?}"))?;
    assert_eq!(source, destination, "directories should be equivalent");

    Ok(())
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct DirectoryMetadata(BTreeMap<RelFilePath, Metadata>);

impl DirectoryMetadata {
    async fn from_directory(root: &AbsDirPath) -> Result<DirectoryMetadata> {
        fs::walk_files(root)
            .map(|entry| async move {
                let entry = entry.context("walk directory")?;
                let metadata = Metadata::from_file(&entry).await.context("get metadata")?;
                let path = entry.relative_to(root).context("make relative")?;
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
