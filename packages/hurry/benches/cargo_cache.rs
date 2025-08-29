//! Benchmarks for caching cargo projects.

use std::path::PathBuf;

use location_macros::workspace_dir;
use tempfile::TempDir;

fn main() {
    divan::main();
}

#[track_caller]
fn setup() -> (PathBuf, TempDir) {
    let workspace = PathBuf::from(workspace_dir!());
    let temp = TempDir::new().expect("create temporary directory");
    (workspace, temp)
}

#[divan::bench(sample_count = 5)]
fn backup() {
    let (_workspace, _temp) = setup();
}

#[divan::bench(sample_count = 5)]
fn restore() {
    let (_workspace, _temp) = setup();
}
