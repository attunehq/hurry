//! Tests for cargo command passthrough functionality.
//!
//! These tests verify that hurry correctly forwards cargo commands and help
//! requests to the underlying cargo binary by comparing the output of
//! `hurry cargo ...` directly with `cargo ...`.

use pretty_assertions::assert_eq as pretty_assert_eq;
use std::process::{Command, Stdio};

/// Helper to run a command and capture its output (both stdout and stderr).
fn run_command(name: &str, args: &[&str]) -> (String, String) {
    let output = Command::new(name)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    (stdout, stderr)
}

/// Test that `hurry cargo build --help` produces the same output as `cargo build --help`.
#[test]
fn build_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "build", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["build", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo build -h` produces the same output as `cargo build -h`.
#[test]
fn build_short_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "build", "-h"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["build", "-h"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo help build` produces the same output as `cargo help build`.
#[test]
fn help_build() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "help", "build"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["help", "build"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo check --help` produces the same output as `cargo check --help`.
#[test]
fn check_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "check", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["check", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo check -h` produces the same output as `cargo check -h`.
#[test]
fn check_short_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "check", "-h"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["check", "-h"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo test --help` produces the same output as `cargo test --help`.
#[test]
fn test_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "test", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["test", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo test -h` produces the same output as `cargo test -h`.
#[test]
fn test_short_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "test", "-h"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["test", "-h"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo help test` produces the same output as `cargo help test`.
#[test]
fn help_test() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "help", "test"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["help", "test"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo clean --help` produces the same output as `cargo clean --help`.
#[test]
fn clean_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "clean", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["clean", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo doc --help` produces the same output as `cargo doc --help`.
#[test]
fn doc_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "doc", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["doc", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo tree --help` produces the same output as `cargo tree --help`.
#[test]
fn tree_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "tree", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["tree", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo version` produces the same output as `cargo version`.
#[test]
fn version() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "version"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["version"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo --version` produces the same output as `cargo --version`.
#[test]
fn version_flag() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "--version"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["--version"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo help` produces the same output as `cargo help`.
#[test]
fn help_no_subcommand() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

/// Test that `hurry cargo run --help` produces the same output as `cargo run --help`.
#[test]
fn run_long_help() {
    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "run", "--help"]);
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["run", "--help"]);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}
