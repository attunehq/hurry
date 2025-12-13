//! Build script for hurry that generates version information.
//!
//! This generates a version string that:
//! - Uses `git describe --always` to get the base version (tag or commit hash)
//! - If the working tree is dirty, appends a content hash of the changed files

use std::hash::{DefaultHasher, Hasher as _};
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

fn main() -> Result<(), String> {
    // We don't emit any `rerun-if-changed` directives because Cargo by default
    // will scan all files in the package directory for changes[^1].
    //
    // This technically means that if you add a _new file_ to the package but
    // you _don't change_ any existing files, the build script will not re-run
    // (because Cargo checks whether previously built files have changed). In
    // practice, this is almost certainly fine, because if the previous build
    // succeeded, and you have not changed any file in the existing build, then
    // adding a new file cannot possibly change the build because the new file
    // cannot possibly have been imported because no existing files changed!
    //
    // The only way this can become a problem is if we start adding files that
    // are added through `include_{str,bytes}!`, which I think Rust has
    // special-case handling for[^2]. In either case, we can escape hatch
    // through `cargo clean`.
    //
    // [^1]: https://doc.rust-lang.org/cargo/reference/build-scripts.html#rerun-if-changed
    // [^2]: https://github.com/rust-lang/cargo/issues/1510
    let version = compute_version()?;
    println!("cargo:rustc-env=HURRY_VERSION={version}");

    Ok(())
}

/// Returns (version_string, list_of_dirty_files)
fn compute_version() -> Result<String, String> {
    // Get base version from git describe.
    let base_version = git_describe()?;

    // Get list of changed files.
    let changed_files = changed_files()?;

    // If this list is empty, return the base version.
    if changed_files.is_empty() {
        return Ok(base_version);
    }

    // Otherwise, calculate the content hash of the changed files.
    let content_hash = content_hash(changed_files)?;

    // Truncate hash to 7 characters like git does for commit hashes.
    let short_hash = &content_hash[..7.min(content_hash.len())];

    Ok(format!("{base_version}-{short_hash}"))
}

fn content_hash(mut files: Vec<StatusEntry>) -> Result<String, String> {
    // Sort lexicographically for stable ordering.
    files.sort();
    files.dedup();

    // Get the repo root to resolve file paths.
    let repo_root = repo_root()?;

    // Compute hash of each file and collect them.
    let mut hashes = Vec::new();
    for file in files {
        let path = Path::new(&repo_root).join(file.path);
        let mut hasher = DefaultHasher::new();
        print!("{}: ", path.display());
        #[allow(
            clippy::disallowed_methods,
            reason = "avoiding extra deps in build script"
        )]
        if let Ok(content) = std::fs::read(&path) {
            hasher.write(path.as_os_str().as_encoded_bytes());
            hasher.write(&content);
            let hash = hasher.finish();
            println!("{hash}");
            hashes.push(hash);
        } else {
            // Skip files that can't be read (e.g. deleted files).
            //
            // TODO: Maybe match this by status?
            println!("skipped");
        }
    }
    hashes.sort();
    hashes.dedup();

    // Compute final hash by hashing all the individual hashes together.
    let mut hasher = DefaultHasher::new();
    for hash in hashes {
        hasher.write_u64(hash);
    }
    let final_hash = hasher.finish();

    Ok(format!("{final_hash:x}"))
}

fn run(prog: &str, argv: &[&str]) -> Result<String, String> {
    let invocation = std::iter::once(prog)
        .chain(argv.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");

    let output = Command::new(prog)
        .args(argv)
        .output()
        .map_err(|e| format!("failed to execute `{invocation}`: {e}"))?;
    if !output.status.success() {
        return Err(format!("`{invocation}` exited with non-zero status"));
    }

    let output = String::from_utf8(output.stdout)
        .map_err(|e| format!("could not parse output of `{invocation}` as UTF-8: {e}"))?;
    Ok(output.trim_end().to_string())
}

fn git_describe() -> Result<String, String> {
    run("git", &["describe", "--always", "--tags", "--dirty=-dirty"])
}

fn repo_root() -> Result<String, String> {
    run("git", &["rev-parse", "--show-toplevel"])
}

fn changed_files() -> Result<Vec<StatusEntry>, String> {
    let output = run("git", &["status", "--porcelain"])?;

    let mut files = Vec::new();
    for line in output.lines() {
        files.push(line.parse::<StatusEntry>()?);
    }

    Ok(files)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum GitFileStatus {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

impl GitFileStatus {
    fn parse(c: char) -> Option<Self> {
        match c {
            ' ' => Some(Self::Unmodified),
            'M' => Some(Self::Modified),
            'A' => Some(Self::Added),
            'D' => Some(Self::Deleted),
            'R' => Some(Self::Renamed),
            'C' => Some(Self::Copied),
            'U' => Some(Self::Unmerged),
            '?' => Some(Self::Untracked),
            '!' => Some(Self::Ignored),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StatusEntry {
    index: GitFileStatus,
    worktree: GitFileStatus,
    path: String,
    // This is used for renames and copies.
    orig_path: Option<String>,
}

impl FromStr for StatusEntry {
    type Err = String;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        if line.len() < 4 {
            return Err("line too short".into());
        }

        let mut chars = line.chars();
        let index_char = chars.next().unwrap();
        let worktree_char = chars.next().unwrap();
        let space = chars.next().unwrap();

        if space != ' ' {
            return Err("expected space after status".into());
        }

        let index = GitFileStatus::parse(index_char)
            .ok_or_else(|| format!("invalid index status: {index_char}"))?;
        let worktree = GitFileStatus::parse(worktree_char)
            .ok_or_else(|| format!("invalid worktree status: {worktree_char}"))?;

        let rest: String = chars.collect();
        let (path, orig_path) = if matches!(index, GitFileStatus::Renamed | GitFileStatus::Copied) {
            // Format: "old_path -> new_path"
            if let Some((old, new)) = rest.split_once(" -> ") {
                (new.to_string(), Some(old.to_string()))
            } else {
                (rest, None)
            }
        } else {
            (rest, None)
        };

        Ok(StatusEntry {
            index,
            worktree,
            path,
            orig_path,
        })
    }
}
