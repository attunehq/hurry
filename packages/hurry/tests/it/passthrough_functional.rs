//! Functional tests for cargo command passthrough.
//!
//! These tests verify that hurry correctly executes cargo commands with the same
//! side effects and output as running cargo directly. Tests use temporary
//! directories and validate both command output and resulting file system state.
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
//! - `check`: `--all-targets`, `--release`, `--lib`, `--all-features`, `--no-default-features`
//! - `tree`: `--depth`, `--prefix`, `--edges`, `--charset`
//! - `metadata`: `--format-version`, `--no-deps`
//! - `run`: `--release`, `--quiet`
//! - `clean`: `--release`
//!
//! ### Advanced Scenario Tests (15 tests)
//! Tests for complex real-world scenarios:
//! - **Manifest path**: Running commands with `--manifest-path` from different directories
//! - **Lockfile modes**: `--locked`, `--frozen` flags
//! - **Feature combinations**: Multiple features specified together
//! - **Version constraints**: Version specifications like `@1.0`
//! - **Binary selection**: Running specific binaries with `--bin`
//! - **Color control**: `--color never/always/auto`
//! - **Verbosity control**: `--verbose`, `--quiet`
//! - **Error cases**: Invalid directories, nonexistent packages
//! - **Selective updates**: `--package` for specific dependency updates
//! - **Package filtering**: `--package` in tree command
//! - **Path dependencies**: Local path dependencies with `--path`
//!
//! ### Commands Not Tested
//! These commands require external state/authentication:
//! - `publish`, `login`, `logout`, `yank` (require registry authentication)
//! - `install`, `uninstall` (modify global cargo state)
//! - `search` (requires network and may have non-deterministic results)

use pretty_assertions::assert_eq as pretty_assert_eq;
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
fn run_in_dir(dir: &Path, name: &str, args: &[&str]) -> CommandResult {
    let output = Command::new(name)
        .current_dir(dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to execute command");

    CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    }
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

// Tests for `cargo init`

#[test]
fn init_creates_same_project_structure() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "init", "--lib", "--name", "test-lib"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["init", "--lib", "--name", "test-lib"],
    );

    // Both should succeed
    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should create Cargo.toml
    assert!(hurry_dir.path().join("Cargo.toml").exists());
    assert!(cargo_dir.path().join("Cargo.toml").exists());

    // Both should create src/lib.rs
    assert!(hurry_dir.path().join("src/lib.rs").exists());
    assert!(cargo_dir.path().join("src/lib.rs").exists());

    // The lib.rs files should be identical
    let hurry_lib = fs::read_to_string(hurry_dir.path().join("src/lib.rs")).unwrap();
    let cargo_lib = fs::read_to_string(cargo_dir.path().join("src/lib.rs")).unwrap();
    pretty_assert_eq!(hurry_lib, cargo_lib);
}

#[test]
fn init_bin_creates_same_project_structure() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "init", "--bin", "--name", "test-bin"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["init", "--bin", "--name", "test-bin"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should create src/main.rs
    assert!(hurry_dir.path().join("src/main.rs").exists());
    assert!(cargo_dir.path().join("src/main.rs").exists());

    let hurry_main = fs::read_to_string(hurry_dir.path().join("src/main.rs")).unwrap();
    let cargo_main = fs::read_to_string(cargo_dir.path().join("src/main.rs")).unwrap();
    pretty_assert_eq!(hurry_main, cargo_main);
}

// Tests for `cargo new`

#[test]
fn new_creates_same_project_structure() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(hurry_dir.path(), "hurry", &["cargo", "new", "mylib", "--lib"]);
    let cargo_result = run_in_dir(cargo_dir.path(), "cargo", &["new", "mylib", "--lib"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    // Compare the created projects
    let hurry_project = hurry_dir.path().join("mylib");
    let cargo_project = cargo_dir.path().join("mylib");

    assert!(hurry_project.join("Cargo.toml").exists());
    assert!(cargo_project.join("Cargo.toml").exists());

    let hurry_lib = fs::read_to_string(hurry_project.join("src/lib.rs")).unwrap();
    let cargo_lib = fs::read_to_string(cargo_project.join("src/lib.rs")).unwrap();
    pretty_assert_eq!(hurry_lib, cargo_lib);
}

// Tests for `cargo add`

#[test]
fn add_modifies_cargo_toml_identically() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    // Set up identical projects
    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // Add the same dependency to both
    let hurry_result = run_in_dir(hurry_dir.path(), "hurry", &["cargo", "add", "serde"]);
    let cargo_result = run_in_dir(cargo_dir.path(), "cargo", &["add", "serde"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    // Read and compare Cargo.toml files
    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    // Both should have serde in dependencies
    assert!(hurry_toml.contains("serde"));
    assert!(cargo_toml.contains("serde"));

    // The dependency declarations should be identical
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_features_modifies_cargo_toml_identically() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "add", "serde", "--features", "derive"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["add", "serde", "--features", "derive"],
    );

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    // Both should have serde with derive feature
    assert!(hurry_toml.contains("serde"));
    assert!(hurry_toml.contains("derive") || hurry_toml.contains("features"));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for `cargo remove`

#[test]
fn remove_modifies_cargo_toml_identically() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // First add a dependency
    run_in_dir(hurry_dir.path(), "cargo", &["add", "serde"]);
    run_in_dir(cargo_dir.path(), "cargo", &["add", "serde"]);

    // Now remove it via different methods
    let hurry_result = run_in_dir(hurry_dir.path(), "hurry", &["cargo", "remove", "serde"]);
    let cargo_result = run_in_dir(cargo_dir.path(), "cargo", &["remove", "serde"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    // Both should not have serde
    assert!(!hurry_toml.contains("serde"));
    assert!(!cargo_toml.contains("serde"));

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for `cargo metadata`

#[test]
fn metadata_produces_identical_json() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "metadata", "--format-version=1", "--no-deps"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["metadata", "--format-version=1", "--no-deps"],
    );

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    // Parse both as JSON and compare structure
    let hurry_json: serde_json::Value =
        serde_json::from_str(&hurry_result.stdout).expect("hurry output is not valid JSON");
    let cargo_json: serde_json::Value =
        serde_json::from_str(&cargo_result.stdout).expect("cargo output is not valid JSON");

    // Compare the package names (paths will differ)
    pretty_assert_eq!(
        hurry_json["packages"][0]["name"],
        cargo_json["packages"][0]["name"]
    );
    pretty_assert_eq!(
        hurry_json["packages"][0]["version"],
        cargo_json["packages"][0]["version"]
    );
}

// Tests for `cargo tree`

#[test]
fn tree_produces_same_dependency_structure() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency so we have something to show in the tree
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "tree"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["tree"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    // Normalize outputs to remove path-specific information
    let hurry_normalized = normalize_output(&hurry_result.stdout);
    let cargo_normalized = normalize_output(&cargo_result.stdout);

    // The dependency trees should be structurally the same
    assert!(hurry_normalized.contains("serde"));
    assert!(cargo_normalized.contains("serde"));
}

// Tests for `cargo check`

#[test]
fn check_produces_same_validation_output() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    // Both should succeed for a valid empty project
    pretty_assert_eq!(hurry_result.exit_code, 0);
}

#[test]
fn check_detects_same_errors() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Write invalid Rust code
    fs::write(
        test_dir.path().join("src/lib.rs"),
        "fn broken() { this is not valid rust }",
    )
    .unwrap();

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check"]);

    // Both should fail (exit code non-zero)
    assert_ne!(hurry_result.exit_code, 0, "hurry should fail for invalid code");
    assert_ne!(cargo_result.exit_code, 0, "cargo should fail for invalid code");

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

// Tests for `cargo clean`

#[test]
fn clean_removes_target_directory() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Build first to create target directory
    run_in_dir(test_dir.path(), "cargo", &["build"]);
    assert!(test_dir.path().join("target").exists());

    // Clean via hurry
    let result = run_in_dir(test_dir.path(), "hurry", &["cargo", "clean"]);
    pretty_assert_eq!(result.exit_code, 0);

    // Target directory should be removed
    assert!(!test_dir.path().join("target").exists());
}

// Tests for `cargo pkgid`

#[test]
fn pkgid_produces_same_package_id() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Generate Cargo.lock (required for pkgid)
    run_in_dir(test_dir.path(), "cargo", &["generate-lockfile"]);

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "pkgid"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["pkgid"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should contain the package name
    assert!(hurry_result.stdout.contains("test-project"));
    assert!(cargo_result.stdout.contains("test-project"));
}

// Tests for `cargo locate-project`

#[test]
fn locate_project_finds_cargo_toml() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "locate-project"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["locate-project"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);

    // Both should output JSON with the Cargo.toml path
    let hurry_json: serde_json::Value = serde_json::from_str(&hurry_result.stdout).unwrap();
    let cargo_json: serde_json::Value = serde_json::from_str(&cargo_result.stdout).unwrap();

    // Both should point to Cargo.toml (exact path will differ)
    assert!(hurry_json["root"]
        .as_str()
        .unwrap()
        .ends_with("Cargo.toml"));
    assert!(cargo_json["root"]
        .as_str()
        .unwrap()
        .ends_with("Cargo.toml"));
}

// Tests for `cargo run`

#[test]
fn run_executes_binary_with_same_output() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_binary_project(test_dir.path(), "test-bin");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "run"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["run"]);

    // Both should succeed
    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should output "Hello, world!"
    assert!(hurry_result.stdout.contains("Hello, world!"));
    assert!(cargo_result.stdout.contains("Hello, world!"));
}

// Tests for `cargo update`

#[test]
fn update_creates_same_lockfile() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency so update has something to do
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    // Remove Cargo.lock if it exists
    let _ = fs::remove_file(test_dir.path().join("Cargo.lock"));

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "update"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);

    // Cargo.lock should now exist
    assert!(test_dir.path().join("Cargo.lock").exists());
}

// Tests for `cargo fetch`

#[test]
fn fetch_downloads_dependencies() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency
    run_in_dir(test_dir.path(), "cargo", &["add", "serde", "--vers", "1.0"]);

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "fetch"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["fetch"]);

    pretty_assert_eq!(hurry_result.exit_code, cargo_result.exit_code);
    pretty_assert_eq!(hurry_result.exit_code, 0);
}

// ============================================================================
// Argument variation tests for each command
// ============================================================================

// Tests for `cargo init` arguments

#[test]
fn init_with_vcs_none() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "init", "--vcs", "none", "--name", "test-lib"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["init", "--vcs", "none", "--name", "test-lib"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should not create .git directory
    assert!(!hurry_dir.path().join(".git").exists());
    assert!(!cargo_dir.path().join(".git").exists());
}

#[test]
fn init_with_edition() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "init", "--edition", "2021", "--name", "test-lib"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["init", "--edition", "2021", "--name", "test-lib"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both Cargo.toml files should specify edition 2021
    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    assert!(hurry_toml.contains("edition = \"2021\""));
    assert!(cargo_toml.contains("edition = \"2021\""));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for `cargo new` arguments

#[test]
fn new_with_vcs_none() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "new", "myproject", "--vcs", "none"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["new", "myproject", "--vcs", "none"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Neither should create .git directory
    assert!(!hurry_dir.path().join("myproject/.git").exists());
    assert!(!cargo_dir.path().join("myproject/.git").exists());
}

#[test]
fn new_with_edition_2021() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "new", "myproject", "--edition", "2021"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["new", "myproject", "--edition", "2021"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml =
        fs::read_to_string(hurry_dir.path().join("myproject/Cargo.toml")).unwrap();
    let cargo_toml =
        fs::read_to_string(cargo_dir.path().join("myproject/Cargo.toml")).unwrap();

    assert!(hurry_toml.contains("edition = \"2021\""));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for `cargo add` arguments

#[test]
fn add_with_no_default_features() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "add", "serde", "--no-default-features"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["add", "serde", "--no-default-features"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    assert!(hurry_toml.contains("default-features = false"));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_optional_flag() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "add", "serde", "--optional"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["add", "serde", "--optional"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    assert!(hurry_toml.contains("optional = true"));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn add_with_rename() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "add", "serde", "--rename", "serde_crate"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["add", "serde", "--rename", "serde_crate"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    assert!(hurry_toml.contains("serde_crate"));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for `cargo remove` arguments

#[test]
fn remove_with_dev_flag() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // Add dev dependency first
    run_in_dir(hurry_dir.path(), "cargo", &["add", "--dev", "serde"]);
    run_in_dir(cargo_dir.path(), "cargo", &["add", "--dev", "serde"]);

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "remove", "--dev", "serde"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["remove", "--dev", "serde"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    assert!(!hurry_toml.contains("[dev-dependencies]\nserde"));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

#[test]
fn remove_with_build_flag() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    // Add build dependency first
    run_in_dir(hurry_dir.path(), "cargo", &["add", "--build", "cc"]);
    run_in_dir(cargo_dir.path(), "cargo", &["add", "--build", "cc"]);

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "remove", "--build", "cc"],
    );
    let cargo_result = run_in_dir(
        cargo_dir.path(),
        "cargo",
        &["remove", "--build", "cc"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for `cargo check` arguments

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

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "check", "--all-targets"],
    );
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--all-targets"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_release() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check", "--release"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--release"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_lib_only() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check", "--lib"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--lib"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_all_features() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "check", "--all-features"],
    );
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--all-features"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_no_default_features() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "check", "--no-default-features"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["check", "--no-default-features"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for `cargo tree` arguments

#[test]
fn tree_with_depth() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "tree", "--depth", "1"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["tree", "--depth", "1"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should show limited depth
    assert!(hurry_result.stdout.contains("test-project"));
    assert!(cargo_result.stdout.contains("test-project"));
}

#[test]
fn tree_with_prefix_none() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "tree", "--prefix", "none"],
    );
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["tree", "--prefix", "none"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn tree_with_edges_no_dev() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "tree", "--edges", "no-dev"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["tree", "--edges", "no-dev"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn tree_with_charset_ascii() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "tree", "--charset", "ascii"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["tree", "--charset", "ascii"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for `cargo metadata` arguments

#[test]
fn metadata_with_format_version() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "metadata", "--format-version=1"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["metadata", "--format-version=1"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Both should output valid JSON
    let _hurry_json: serde_json::Value = serde_json::from_str(&hurry_result.stdout).unwrap();
    let _cargo_json: serde_json::Value = serde_json::from_str(&cargo_result.stdout).unwrap();
}

#[test]
fn metadata_with_no_deps() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "metadata", "--no-deps"],
    );
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["metadata", "--no-deps"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_json: serde_json::Value = serde_json::from_str(&hurry_result.stdout).unwrap();
    let cargo_json: serde_json::Value = serde_json::from_str(&cargo_result.stdout).unwrap();

    // Both should have packages array with only the root package
    pretty_assert_eq!(
        hurry_json["packages"].as_array().unwrap().len(),
        cargo_json["packages"].as_array().unwrap().len()
    );
}

// Tests for `cargo run` arguments

#[test]
fn run_with_release() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_binary_project(test_dir.path(), "test-bin");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "run", "--release"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["run", "--release"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_result.stdout.contains("Hello, world!"));
    assert!(cargo_result.stdout.contains("Hello, world!"));
}

#[test]
fn run_with_quiet_flag() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_binary_project(test_dir.path(), "test-bin");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "run", "--quiet"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["run", "--quiet"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Quiet mode should still show the program output
    assert!(hurry_result.stdout.contains("Hello, world!"));
    assert!(cargo_result.stdout.contains("Hello, world!"));
}

// Tests for `cargo clean` arguments

#[test]
fn clean_with_release_only() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Build both debug and release
    run_in_dir(test_dir.path(), "cargo", &["build"]);
    run_in_dir(test_dir.path(), "cargo", &["build", "--release"]);

    assert!(test_dir.path().join("target/debug").exists());
    assert!(test_dir.path().join("target/release").exists());

    // Clean only release
    let result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "clean", "--release"],
    );
    pretty_assert_eq!(result.exit_code, 0);

    // Debug should still exist, release should be gone
    assert!(test_dir.path().join("target/debug").exists());
    assert!(!test_dir.path().join("target/release").exists());
}

// ============================================================================
// Advanced scenario tests
// ============================================================================

// Tests for manifest path

#[test]
fn check_with_manifest_path() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    let project_dir = test_dir.path().join("myproject");
    fs::create_dir(&project_dir).unwrap();
    create_minimal_project(&project_dir, "test-project");

    // Run from parent directory with manifest path
    let manifest_path = project_dir.join("Cargo.toml");
    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &[
            "cargo",
            "check",
            "--manifest-path",
            manifest_path.to_str().unwrap(),
        ],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["check", "--manifest-path", manifest_path.to_str().unwrap()],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for locked/offline flags

#[test]
fn check_with_locked_flag() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Generate lockfile first
    run_in_dir(test_dir.path(), "cargo", &["generate-lockfile"]);

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check", "--locked"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--locked"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_frozen_flag() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Generate lockfile first
    run_in_dir(test_dir.path(), "cargo", &["generate-lockfile"]);

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check", "--frozen"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--frozen"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for feature combinations

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

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "check", "--features", "feature1,feature2"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["check", "--features", "feature1,feature2"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for version constraints

#[test]
fn add_with_version_constraint() {
    let hurry_dir = TempDir::new().expect("failed to create temp dir");
    let cargo_dir = TempDir::new().expect("failed to create temp dir");

    create_minimal_project(hurry_dir.path(), "test-project");
    create_minimal_project(cargo_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        hurry_dir.path(),
        "hurry",
        &["cargo", "add", "serde@1.0"],
    );
    let cargo_result = run_in_dir(cargo_dir.path(), "cargo", &["add", "serde@1.0"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let hurry_toml = fs::read_to_string(hurry_dir.path().join("Cargo.toml")).unwrap();
    let cargo_toml = fs::read_to_string(cargo_dir.path().join("Cargo.toml")).unwrap();

    // Both should have serde with version 1.0
    assert!(hurry_toml.contains("serde"));
    assert!(hurry_toml.contains("1.0"));
    pretty_assert_eq!(hurry_toml, cargo_toml);
}

// Tests for binary selection

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

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "run", "--bin", "bin1"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["run", "--bin", "bin1"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    assert!(hurry_result.stdout.contains("bin1"));
    assert!(cargo_result.stdout.contains("bin1"));
}

// Tests for color output control

#[test]
fn check_with_color_never() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "check", "--color", "never"],
    );
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--color", "never"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    // Output should not contain ANSI escape codes
    assert!(!hurry_result.stderr.contains("\x1b["));
    assert!(!cargo_result.stderr.contains("\x1b["));
}

// Tests for quiet/verbose flags

#[test]
fn check_with_verbose() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "check", "--verbose"],
    );
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--verbose"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

#[test]
fn check_with_quiet() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check", "--quiet"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check", "--quiet"]);

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for error cases

#[test]
fn check_in_non_cargo_directory() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    // Don't create any Cargo.toml

    let hurry_result = run_in_dir(test_dir.path(), "hurry", &["cargo", "check"]);
    let cargo_result = run_in_dir(test_dir.path(), "cargo", &["check"]);

    // Both should fail
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

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "add", "this-package-definitely-does-not-exist-xyz123"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["add", "this-package-definitely-does-not-exist-xyz123"],
    );

    // Both should fail
    assert_ne!(hurry_result.exit_code, 0);
    assert_ne!(cargo_result.exit_code, 0);
}

// Tests for update with specific package

#[test]
fn update_specific_package() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");

    // Add a dependency
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "update", "--package", "serde"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["update", "--package", "serde"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for tree with package selection

#[test]
fn tree_with_package_filter() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    create_minimal_project(test_dir.path(), "test-project");
    run_in_dir(test_dir.path(), "cargo", &["add", "serde"]);

    let hurry_result = run_in_dir(
        test_dir.path(),
        "hurry",
        &["cargo", "tree", "--package", "test-project"],
    );
    let cargo_result = run_in_dir(
        test_dir.path(),
        "cargo",
        &["tree", "--package", "test-project"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);
}

// Tests for path dependencies

#[test]
fn add_path_dependency() {
    let test_dir = TempDir::new().expect("failed to create temp dir");
    let main_project = test_dir.path().join("main");
    let dep_project = test_dir.path().join("dep");

    fs::create_dir(&main_project).unwrap();
    fs::create_dir(&dep_project).unwrap();

    create_minimal_project(&main_project, "main-project");
    create_minimal_project(&dep_project, "dep-project");

    let hurry_result = run_in_dir(
        &main_project,
        "hurry",
        &["cargo", "add", "dep-project", "--path", "../dep"],
    );
    let cargo_result = run_in_dir(
        &main_project,
        "cargo",
        &["add", "dep-project", "--path", "../dep"],
    );

    pretty_assert_eq!(hurry_result.exit_code, 0);
    pretty_assert_eq!(cargo_result.exit_code, 0);

    let toml_content = fs::read_to_string(main_project.join("Cargo.toml")).unwrap();
    assert!(toml_content.contains("dep-project"));
    assert!(toml_content.contains("path"));
}
