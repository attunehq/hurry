//! Build script for hurry that generates version information.
//!
//! This generates a version string that:
//! - Uses `git describe --always` to get the base version (tag or commit hash)
//! - If the working tree is dirty, appends a content hash of the changed files
//!
//! The content hash is computed by:
//! 1. Getting the list of changed files from `git diff --name-only`
//! 2. Sorting them lexicographically
//! 3. Computing blake3 hash of each file's content
//! 4. Computing a final blake3 hash of all the individual hashes

use std::path::Path;
use std::process::Command;

fn main() {
    // Rerun if any git state changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let (version, dirty_files) = compute_version();
    println!("cargo:rustc-env=HURRY_VERSION={version}");

    // Emit rerun-if-changed for all dirty files so we rebuild when they change
    if let Some(repo_root) = get_repo_root() {
        for file in dirty_files {
            let path = Path::new(&repo_root).join(&file);
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// Returns (version_string, list_of_dirty_files)
fn compute_version() -> (String, Vec<String>) {
    // Get base version from git describe
    let base_version = git_describe().unwrap_or_else(|| String::from("unknown"));

    // Check if tree is dirty
    if !is_tree_dirty() {
        return (base_version, Vec::new());
    }

    // Compute content hash of dirty files
    let (dirty_hash, dirty_files) = match compute_dirty_hash() {
        Some((hash, files)) => (hash, files),
        None => return (format!("{base_version}-dirty"), Vec::new()),
    };

    // Truncate hash to 7 characters like git does for commit hashes
    let short_hash = &dirty_hash[..7.min(dirty_hash.len())];
    (format!("{base_version}-{short_hash}"), dirty_files)
}

fn git_describe() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--always", "--tags"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8(output.stdout).ok()?;
    Some(version.trim().to_string())
}

fn is_tree_dirty() -> bool {
    let output = Command::new("git").args(["status", "--porcelain"]).output();

    match output {
        Ok(output) => !output.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Returns (hash, list_of_dirty_files)
fn compute_dirty_hash() -> Option<(String, Vec<String>)> {
    // Get list of changed files (both staged and unstaged)
    let output = Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let diff_output = String::from_utf8(output.stdout).ok()?;
    let mut changed_files = diff_output
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect::<Vec<_>>();

    // Also get untracked files
    let untracked_output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .ok()?;

    if untracked_output.status.success() {
        if let Ok(untracked) = String::from_utf8(untracked_output.stdout) {
            for line in untracked.lines() {
                if !line.is_empty() {
                    changed_files.push(String::from(line));
                }
            }
        }
    }

    if changed_files.is_empty() {
        return None;
    }

    // Sort lexicographically for stable ordering
    changed_files.sort();
    changed_files.dedup();

    // Get the repo root to resolve file paths
    let repo_root = get_repo_root()?;

    // Compute blake3 hash of each file and collect them
    let mut file_hashes = Vec::new();
    for file in &changed_files {
        let path = Path::new(&repo_root).join(file);
        if let Ok(content) = std::fs::read(&path) {
            let hash = blake3::hash(&content);
            file_hashes.push(hash);
        }
        // Skip files that can't be read (e.g., deleted files)
    }

    if file_hashes.is_empty() {
        return None;
    }

    // Compute final hash by hashing all the individual hashes together
    let mut hasher = blake3::Hasher::new();
    for hash in &file_hashes {
        hasher.update(hash.as_bytes());
    }
    let final_hash = hasher.finalize();

    Some((final_hash.to_hex().to_string(), changed_files))
}

fn get_repo_root() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let root = String::from_utf8(output.stdout).ok()?;
    Some(root.trim().to_string())
}
