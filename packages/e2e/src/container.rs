use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    process::Command,
};

use color_eyre::{
    Result,
    eyre::{Context, bail},
};

/// Get a unique identifier for the current working tree state.
///
/// This returns a string that uniquely identifies the current state of the
/// repository, including uncommitted changes. This is useful for cache
/// invalidation: the returned value changes whenever the working tree content
/// changes, ensuring that cached artifacts are rebuilt when code changes.
///
/// The returned format is:
/// - Clean tree: `"abc1234"` (short commit SHA)
/// - Dirty tree: `"abc1234-f1e2d3c4b5a6"` (commit SHA + diff hash)
///
/// This prevents surprising behavior where making changes doesn't trigger a
/// rebuild until after committing. With this approach, any content change to
/// tracked files will be detected, and each unique working tree state can be
/// cached separately.
///
/// # Example
/// ```ignore
/// let tag = working_tree_hash(&workspace_root)?;
/// // tag might be "abc1234" or "abc1234-f1e2d3c4b5a6"
/// let image_name = format!("my-image:{tag}");
/// ```
pub(crate) fn working_tree_hash(root: &Path) -> Result<String> {
    let commit_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .context("execute git rev-parse")?;
    if !commit_sha.status.success() {
        bail!("git rev-parse failed with status: {}", commit_sha.status);
    }

    let sha = String::from_utf8(commit_sha.stdout)
        .context("parse git SHA as UTF-8")?
        .trim()
        .to_string();

    // Check if there are any uncommitted changes (staged or unstaged).
    // We use `git diff HEAD` so that we hash the content of the changes,
    // not just the filenames.
    let git_diff = Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(root)
        .output()
        .context("execute git diff")?;
    if !git_diff.status.success() {
        bail!("git diff failed with status: {}", git_diff.status);
    }

    if !git_diff.stdout.is_empty() {
        let mut hasher = DefaultHasher::new();
        git_diff.stdout.hash(&mut hasher);
        let dirty_hash = hasher.finish();
        Ok(format!("{sha}-{dirty_hash:x}"))
    } else {
        Ok(sha)
    }
}
