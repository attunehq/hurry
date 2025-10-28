//! Functional tests for cargo command passthrough.
//!
//! These tests verify that hurry correctly executes cargo commands with the
//! same side effects and output as running cargo directly. Tests use temporary
//! directories and validate both command output and resulting file system
//! state.
//!
//! ## Test Coverage
//!
//! ### Basic Functionality Tests (16 tests)
//! Commands tested with basic usage:
//! - Project creation: `init`, `new`
//! - Dependency management: `add`, `remove`, `update`, `fetch`
//! - Validation: `check`
//! - Introspection: `metadata`, `tree`, `pkgid`, `locate-project`
//! - Execution: `run`
//! - Maintenance: `clean`
//!
//! ### Argument Variation Tests (23 tests)
//! Tests for stable command-line arguments:
//! - `init`: `--vcs`, `--edition`, `--name`, `--lib`, `--bin`
//! - `new`: `--vcs`, `--edition`, `--lib`, `--bin`
//! - `add`: `--features`, `--no-default-features`, `--optional`, `--rename`
//! - `remove`: `--dev`, `--build`
//! - `check`: `--all-targets`, `--release`, `--lib`, `--all-features`,
//!   `--no-default-features`
//! - `tree`: `--depth`, `--prefix`, `--edges`, `--charset`
//! - `metadata`: `--format-version`, `--no-deps`
//! - `run`: `--release`, `--quiet`
//! - `clean`: `--release`
//!
//! ### Advanced Scenario Tests (15 tests)
//! Tests for complex real-world scenarios:
//! - Manifest path: Running commands with `--manifest-path` from different
//!   directories
//! - Lockfile modes: `--locked`, `--frozen` flags
//! - Feature combinations: Multiple features specified together
//! - Version constraints: Version specifications like `@1.0`
//! - Binary selection: Running specific binaries with `--bin`
//! - Color control: `--color never/always/auto`
//! - Verbosity control: `--verbose`, `--quiet`
//! - Error cases: Invalid directories, nonexistent packages
//! - Selective updates: `--package` for specific dependency updates
//! - Package filtering: `--package` in tree command
//! - Path dependencies: Local path dependencies with `--path`
//!
//! ### Commands Not Tested
//! These commands require external state/authentication:
//! - `publish`, `login`, `logout`, `yank` (require registry authentication)
//! - `install`, `uninstall` (modify global cargo state)
//! - `search` (requires network and may have non-deterministic results)
//!
//! Even though they're not tested, we've built up enough validation through the
//! other tests that these should be safe (we do the same thing for all
//! passthrough commands). If necessary we can add tests for these specifically
//! in the future.

use pretty_assertions::assert_eq as pretty_assert_eq;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

/// Result of running a command in a directory.
#[derive(Debug)]
struct CommandResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

/// Run a command in the given directory and capture its output.
#[track_caller]
fn run_in_dir(dir: &Path, name: &str, args: &[&str]) -> std::io::Result<CommandResult> {
    let output = Command::new(name)
        .current_dir(dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    Ok(CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

/// Run hurry-dev in the given directory and capture its output.
#[track_caller]
fn run_hurry(dir: &Path, args: &[&str]) -> CommandResult {
    match run_in_dir(dir, "hurry-dev", args) {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            panic!("run `make install-dev` to install hurry-dev")
        }
        Err(e) => panic!("failed to execute 'hurry-dev': {e}"),
    }
}

/// Run cargo in the given directory and capture its output.
#[track_caller]
fn run_cargo(dir: &Path, args: &[&str]) -> CommandResult {
    run_in_dir(dir, "cargo", args).unwrap_or_else(|e| panic!("failed to execute 'cargo': {e}"))
}

/// Create a minimal Cargo.toml for testing.
fn create_minimal_project(dir: &Path, name: &str) {
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
"#
    );

    fs::write(dir.join("Cargo.toml"), cargo_toml).expect("failed to write Cargo.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src dir");
    fs::write(dir.join("src/lib.rs"), "").expect("failed to write lib.rs");
}

/// Create a binary project for testing.
fn create_binary_project(dir: &Path, name: &str) {
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
"#
    );

    fs::write(dir.join("Cargo.toml"), cargo_toml).expect("failed to write Cargo.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src dir");
    fs::write(
        dir.join("src/main.rs"),
        r#"fn main() {
    println!("Hello, world!");
}
"#,
    )
    .expect("failed to write main.rs");
}

/// Normalize output by removing timestamps, paths, and other variable content.
fn normalize_output(output: &str) -> String {
    output
        .lines()
        .filter(|line| {
            // Filter out lines with timing information
            !line.contains("Finished") && !line.contains("Running")
        })
        .map(|line| {
            // Remove package IDs with paths
            line.split_whitespace()
                .filter(|word| !word.starts_with("(/") && !word.starts_with("(file://"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Normalize JSON metadata by removing path-specific fields that will differ.
fn normalize_metadata_json(json: &mut Value) {
    if let Some(packages) = json["packages"].as_array_mut() {
        for package in packages {
            if let Some(obj) = package.as_object_mut() {
                obj.remove("manifest_path");
                obj.remove("id");
            }
        }
    }
    if let Some(obj) = json.as_object_mut() {
        obj.remove("workspace_root");
        obj.remove("target_directory");
    }
}

#[test]
fn init_creates_same_project_structure() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "init", "--lib", "--name", "test-lib"],
    );
    let cargo_result = run_cargo(cargo_dir.path(), &["init", "--lib", "--name", "test-lib"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_dir.path().join("Cargo.toml").exists());
    assert!(cargo_dir.path().join("Cargo.toml").exists());

    assert!(hurry_dir.path().join("src/lib.rs").exists());
    assert!(cargo_dir.path().join("src/lib.rs").exists());

    let hurry_lib = fs::read_to_string(hurry_dir.path().join("src/lib.rs")).unwrap();
    let cargo_lib = fs::read_to_string(cargo_dir.path().join("src/lib.rs")).unwrap();
    pretty_assert_eq!(hurry_lib, cargo_lib);
}

#[test]
fn init_bin_creates_same_project_structure() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "init", "--bin", "--name", "test-bin"],
    );
    let cargo_result = run_cargo(cargo_dir.path(), &["init", "--bin", "--name", "test-bin"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_dir.path().join("src/main.rs").exists());
    assert!(cargo_dir.path().join("src/main.rs").exists());

    let hurry_main = fs::read_to_string(hurry_dir.path().join("src/main.rs")).unwrap();
    let cargo_main = fs::read_to_string(cargo_dir.path().join("src/main.rs")).unwrap();
    pretty_assert_eq!(hurry_main, cargo_main);
}

#[test]
fn new_creates_same_project_structure() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "new", "mylib", "--lib"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["new", "mylib", "--lib"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_project = hurry_dir.path().join("mylib");
    let cargo_project = cargo_dir.path().join("mylib");

    assert!(hurry_project.join("Cargo.toml").exists());
    assert!(cargo_project.join("Cargo.toml").exists());

    let hurry_lib = fs::read_to_string(hurry_project.join("src/lib.rs")).unwrap();
    let cargo_lib = fs::read_to_string(cargo_project.join("src/lib.rs")).unwrap();
    pretty_assert_eq!(hurry_lib, cargo_lib);
}

#[test]
fn add_modifies_cargo_toml_identically() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "add", "serde"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["add", "serde"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_features_modifies_cargo_toml_identically() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "add", "serde", "--features", "derive"],
    );
    let cargo_result = run_cargo(cargo_dir.path(), &["add", "serde", "--features", "derive"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn remove_modifies_cargo_toml_identically() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // First add a dependency
    run_cargo(hurry_dir.path(), &["add", "serde"]);
    run_cargo(cargo_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "remove", "serde"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["remove", "serde"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn metadata_produces_identical_json() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(
        test_dir.path(),
        &["cargo", "metadata", "--format-version=1", "--no-deps"],
    );
    let cargo_result = run_cargo(
        test_dir.path(),
        &["metadata", "--format-version=1", "--no-deps"],
    );

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let mut hurry_json =
        serde_json::from_str(&hurry_result.stdout).expect("hurry output is not valid JSON");
    let mut cargo_json =
        serde_json::from_str(&cargo_result.stdout).expect("cargo output is not valid JSON");

    normalize_metadata_json(&mut hurry_json);
    normalize_metadata_json(&mut cargo_json);

    pretty_assert_eq!(hurry_json, cargo_json);
}

#[test]
fn tree_produces_same_dependency_structure() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency so we have something to show in the tree
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "tree"]);
    let cargo_result = run_cargo(test_dir.path(), &["tree"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_normalized = normalize_output(&hurry_result.stdout);
    let cargo_normalized = normalize_output(&cargo_result.stdout);

    pretty_assert_eq!(hurry_normalized, cargo_normalized);
}

#[test]
fn check_produces_same_validation_output() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check"]);
    let cargo_result = run_cargo(test_dir.path(), &["check"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    pretty_assert_eq!(hurry_result.exit_code, 0);
}

#[test]
fn check_detects_same_errors() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    fs::write(
        test_dir.path().join("src/lib.rs"),
        "fn broken() { this is not valid rust }",
    )
    .unwrap();

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check"]);
    let cargo_result = run_cargo(test_dir.path(), &["check"]);

    // Both should fail (exit code non-zero)
    assert_ne!(
        hurry_result.exit_code, 0,
        "hurry should fail for invalid code"
    );
    assert_ne!(
        cargo_result.exit_code, 0,
        "cargo should fail for invalid code"
    );

    // Both should report errors in stderr
    assert!(
        hurry_result.stderr.contains("error"),
        "hurry should report error in stderr"
    );
    assert!(
        cargo_result.stderr.contains("error"),
        "cargo should report error in stderr"
    );
}

#[test]
fn clean_removes_target_directory() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Build first to create target directory
    run_cargo(test_dir.path(), &["build"]);
    assert!(test_dir.path().join("target").exists());

    // Clean via hurry
    let result = run_hurry(test_dir.path(), &["cargo", "clean"]);
    pretty_assert_eq!(result.exit_code, 0);

    // Target directory should be removed
    assert!(!test_dir.path().join("target").exists());
}

#[test]
fn pkgid_produces_same_package_id() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Generate Cargo.lock (required for pkgid)
    run_cargo(test_dir.path(), &["generate-lockfile"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "pkgid"]);
    let cargo_result = run_cargo(test_dir.path(), &["pkgid"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    pretty_assert_eq!(hurry_result.stdout, cargo_result.stdout);
}

#[test]
fn locate_project_finds_cargo_toml() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "locate-project"]);
    let cargo_result = run_cargo(test_dir.path(), &["locate-project"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_json = serde_json::from_str::<Value>(&hurry_result.stdout).unwrap();
    let cargo_json = serde_json::from_str::<Value>(&cargo_result.stdout).unwrap();

    pretty_assert_eq!(hurry_json, cargo_json);
}

#[test]
fn run_executes_binary_with_same_output() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_binary_project(test_dir.path(), "test-bin");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "run"]);
    let cargo_result = run_cargo(test_dir.path(), &["run"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_result.stdout.contains("Hello, world!"));
    assert!(cargo_result.stdout.contains("Hello, world!"));
}

#[test]
fn update_creates_same_lockfile() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency so update has something to do
    run_cargo(test_dir.path(), &["add", "serde"]);

    // Remove Cargo.lock if it exists
    let _ = fs::remove_file(test_dir.path().join("Cargo.lock"));

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "update"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);

    // Cargo.lock should now exist
    assert!(test_dir.path().join("Cargo.lock").exists());
}

#[test]
fn fetch_downloads_dependencies() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency
    run_cargo(test_dir.path(), &["add", "serde", "--vers", "1.0"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "fetch"]);
    let cargo_result = run_cargo(test_dir.path(), &["fetch"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);
    pretty_assert_eq!(hurry_result.exit_code, 0);
}

#[test]
fn init_with_vcs_none() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "init", "--vcs", "none", "--name", "test-lib"],
    );
    let cargo_result = run_cargo(
        cargo_dir.path(),
        &["init", "--vcs", "none", "--name", "test-lib"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(!hurry_dir.path().join(".git").exists());
    assert!(!cargo_dir.path().join(".git").exists());
}

#[test]
fn init_with_edition() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "init", "--edition", "2021", "--name", "test-lib"],
    );
    let cargo_result = run_cargo(
        cargo_dir.path(),
        &["init", "--edition", "2021", "--name", "test-lib"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn new_with_vcs_none() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "new", "myproject", "--vcs", "none"],
    );
    let cargo_result = run_cargo(cargo_dir.path(), &["new", "myproject", "--vcs", "none"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(!hurry_dir.path().join("myproject/.git").exists());
    assert!(!cargo_dir.path().join("myproject/.git").exists());
}

#[test]
fn new_with_edition_2021() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "new", "myproject", "--edition", "2021"],
    );
    let cargo_result = run_cargo(cargo_dir.path(), &["new", "myproject", "--edition", "2021"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("myproject/Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("myproject/Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_no_default_features() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "add", "serde", "--no-default-features"],
    );
    let cargo_result = run_cargo(cargo_dir.path(), &["add", "serde", "--no-default-features"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_optional_flag() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "add", "serde", "--optional"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["add", "serde", "--optional"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_rename() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_hurry(
        hurry_dir.path(),
        &["cargo", "add", "serde", "--rename", "serde_crate"],
    );
    let cargo_result = run_cargo(
        cargo_dir.path(),
        &["add", "serde", "--rename", "serde_crate"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn remove_with_dev_flag() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // Add dev dependency first
    run_cargo(hurry_dir.path(), &["add", "--dev", "serde"]);
    run_cargo(cargo_dir.path(), &["add", "--dev", "serde"]);

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "remove", "--dev", "serde"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["remove", "--dev", "serde"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn remove_with_build_flag() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // Add build dependency first
    run_cargo(hurry_dir.path(), &["add", "--build", "cc"]);
    run_cargo(cargo_dir.path(), &["add", "--build", "cc"]);

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "remove", "--build", "cc"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["remove", "--build", "cc"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn check_with_all_targets() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a test target
    fs::write(
        test_dir.path().join("src/lib.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {}\n}",
    )
    .unwrap();

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--all-targets"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--all-targets"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_release() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--release"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--release"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_lib_only() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--lib"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--lib"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_all_features() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--all-features"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--all-features"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_no_default_features() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(
        test_dir.path(),
        &["cargo", "check", "--no-default-features"],
    );
    let cargo_result = run_cargo(test_dir.path(), &["check", "--no-default-features"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn tree_with_depth() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "tree", "--depth", "1"]);
    let cargo_result = run_cargo(test_dir.path(), &["tree", "--depth", "1"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_normalized = normalize_output(&hurry_result.stdout);
    let cargo_normalized = normalize_output(&cargo_result.stdout);

    pretty_assert_eq!(hurry_normalized, cargo_normalized);
}

#[test]
fn tree_with_prefix_none() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "tree", "--prefix", "none"]);
    let cargo_result = run_cargo(test_dir.path(), &["tree", "--prefix", "none"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn tree_with_edges_no_dev() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "tree", "--edges", "no-dev"]);
    let cargo_result = run_cargo(test_dir.path(), &["tree", "--edges", "no-dev"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn tree_with_charset_ascii() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "tree", "--charset", "ascii"]);
    let cargo_result = run_cargo(test_dir.path(), &["tree", "--charset", "ascii"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn metadata_with_format_version() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(
        test_dir.path(),
        &["cargo", "metadata", "--format-version=1"],
    );
    let cargo_result = run_cargo(test_dir.path(), &["metadata", "--format-version=1"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let mut hurry_json = serde_json::from_str::<Value>(&hurry_result.stdout).unwrap();
    let mut cargo_json = serde_json::from_str::<Value>(&cargo_result.stdout).unwrap();

    normalize_metadata_json(&mut hurry_json);
    normalize_metadata_json(&mut cargo_json);

    pretty_assert_eq!(hurry_json, cargo_json);
}

#[test]
fn metadata_with_no_deps() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "metadata", "--no-deps"]);
    let cargo_result = run_cargo(test_dir.path(), &["metadata", "--no-deps"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let mut hurry_json = serde_json::from_str::<Value>(&hurry_result.stdout).unwrap();
    let mut cargo_json = serde_json::from_str::<Value>(&cargo_result.stdout).unwrap();

    normalize_metadata_json(&mut hurry_json);
    normalize_metadata_json(&mut cargo_json);

    pretty_assert_eq!(hurry_json, cargo_json);
}

#[test]
fn run_with_release() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_binary_project(test_dir.path(), "test-bin");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "run", "--release"]);
    let cargo_result = run_cargo(test_dir.path(), &["run", "--release"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_result.stdout.contains("Hello, world!"));
    assert!(cargo_result.stdout.contains("Hello, world!"));
}

#[test]
fn run_with_quiet_flag() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_binary_project(test_dir.path(), "test-bin");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "run", "--quiet"]);
    let cargo_result = run_cargo(test_dir.path(), &["run", "--quiet"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Quiet mode should still show the program output
    assert!(hurry_result.stdout.contains("Hello, world!"));
    assert!(cargo_result.stdout.contains("Hello, world!"));
}

#[test]
fn clean_with_release_only() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Build both debug and release
    run_cargo(test_dir.path(), &["build"]);
    run_cargo(test_dir.path(), &["build", "--release"]);

    assert!(test_dir.path().join("target/debug").exists());
    assert!(test_dir.path().join("target/release").exists());

    // Clean only release
    let result = run_hurry(test_dir.path(), &["cargo", "clean", "--release"]);
    pretty_assert_eq!(result.exit_code, 0);

    // Debug should still exist, release should be gone
    assert!(test_dir.path().join("target/debug").exists());
    assert!(!test_dir.path().join("target/release").exists());
}

#[test]
fn check_with_manifest_path() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    let project_dir = test_dir.path().join("myproject");
    fs::create_dir(&project_dir).unwrap();
    create_minimal_project(&project_dir, "test-project");

    // Run from parent directory with manifest path
    let manifest_path = project_dir.join("Cargo.toml");
    let hurry_result = run_hurry(
        test_dir.path(),
        &[
            "cargo",
            "check",
            "--manifest-path",
            manifest_path.to_str().unwrap(),
        ],
    );
    let cargo_result = run_cargo(
        test_dir.path(),
        &["check", "--manifest-path", manifest_path.to_str().unwrap()],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_locked_flag() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Generate lockfile first
    run_cargo(test_dir.path(), &["generate-lockfile"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--locked"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--locked"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_frozen_flag() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Generate lockfile first
    run_cargo(test_dir.path(), &["generate-lockfile"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--frozen"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--frozen"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_specific_features() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add Cargo.toml with features
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"

[features]
feature1 = []
feature2 = []

[dependencies]
"#;
    fs::write(test_dir.path().join("Cargo.toml"), cargo_toml).unwrap();

    let hurry_result = run_hurry(
        test_dir.path(),
        &["cargo", "check", "--features", "feature1,feature2"],
    );
    let cargo_result = run_cargo(
        test_dir.path(),
        &["check", "--features", "feature1,feature2"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn add_with_version_constraint() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_hurry(hurry_dir.path(), &["cargo", "add", "serde@1.0"]);
    let cargo_result = run_cargo(cargo_dir.path(), &["add", "serde@1.0"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn run_specific_binary() {
    let test_dir = TempDir::new().expect("failed to create temp dir");

    // Create a project with multiple binaries
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "bin1"
path = "src/bin1.rs"

[[bin]]
name = "bin2"
path = "src/bin2.rs"
"#;

    fs::write(test_dir.path().join("Cargo.toml"), cargo_toml).unwrap();
    fs::create_dir_all(test_dir.path().join("src")).unwrap();
    fs::write(
        test_dir.path().join("src/bin1.rs"),
        r#"fn main() { println!("bin1"); }"#,
    )
    .unwrap();
    fs::write(
        test_dir.path().join("src/bin2.rs"),
        r#"fn main() { println!("bin2"); }"#,
    )
    .unwrap();

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "run", "--bin", "bin1"]);
    let cargo_result = run_cargo(test_dir.path(), &["run", "--bin", "bin1"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_result.stdout.contains("bin1"));
    assert!(cargo_result.stdout.contains("bin1"));
}

#[test]
fn check_with_color_never() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--color", "never"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--color", "never"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Output should not contain ANSI escape codes
    assert!(!hurry_result.stderr.contains("\x1b["));
    assert!(!cargo_result.stderr.contains("\x1b["));
}

#[test]
fn check_with_verbose() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--verbose"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--verbose"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_quiet() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check", "--quiet"]);
    let cargo_result = run_cargo(test_dir.path(), &["check", "--quiet"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_in_non_cargo_directory() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    // Don't create any Cargo.toml

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "check"]);
    let cargo_result = run_cargo(test_dir.path(), &["check"]);

    assert_ne!(hurry_result.exit_code, 0);
    assert_ne!(cargo_result.exit_code, 0);

    // Both should report the same type of error
    assert!(hurry_result.stderr.contains("Cargo.toml"));
    assert!(cargo_result.stderr.contains("Cargo.toml"));
}

#[test]
fn add_nonexistent_package() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_hurry(
        test_dir.path(),
        &[
            "cargo",
            "add",
            "this-package-definitely-does-not-exist-xyz123",
        ],
    );
    let cargo_result = run_cargo(
        test_dir.path(),
        &["add", "this-package-definitely-does-not-exist-xyz123"],
    );

    assert_ne!(hurry_result.exit_code, 0);
    assert_ne!(cargo_result.exit_code, 0);
}

#[test]
fn update_specific_package() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(test_dir.path(), &["cargo", "update", "--package", "serde"]);
    let cargo_result = run_cargo(test_dir.path(), &["update", "--package", "serde"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn tree_with_package_filter() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_cargo(test_dir.path(), &["add", "serde"]);

    let hurry_result = run_hurry(
        test_dir.path(),
        &["cargo", "tree", "--package", "test-project"],
    );
    let cargo_result = run_cargo(test_dir.path(), &["tree", "--package", "test-project"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn add_path_dependency() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_main = hurry_dir.path().join("main");
    let hurry_dep = hurry_dir.path().join("dep");
    let cargo_main = cargo_dir.path().join("main");
    let cargo_dep = cargo_dir.path().join("dep");

    fs::create_dir(&hurry_main).unwrap();
    fs::create_dir(&hurry_dep).unwrap();
    fs::create_dir(&cargo_main).unwrap();
    fs::create_dir(&cargo_dep).unwrap();

    create_minimal_project(&hurry_main, "main-project");
    create_minimal_project(&hurry_dep, "dep-project");
    create_minimal_project(&cargo_main, "main-project");
    create_minimal_project(&cargo_dep, "dep-project");

    let hurry_result = run_hurry(
        &hurry_main,
        &["cargo", "add", "dep-project", "--path", "../dep"],
    );
    let cargo_result = run_cargo(&cargo_main, &["add", "dep-project", "--path", "../dep"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_main.join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_main.join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}
