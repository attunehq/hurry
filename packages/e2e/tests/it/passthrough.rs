//! Tests for cargo command passthrough functionality.
//!
//! These tests verify that hurry correctly forwards cargo commands and help
//! requests to the underlying cargo binary by comparing the output of
//! `hurry cargo ...` directly with `cargo ...`.

use color_eyre::Result;
use e2e::temporary_directory;
use pretty_assertions::assert_eq as pretty_assert_eq;
use std::process::{Command, Stdio};

/// Helper to run a command and capture its output (both stdout and stderr).
fn run_command(name: &str, args: &[&str], pwd: &std::path::Path) -> Result<(String, String)> {
    let output = Command::new(name)
        .current_dir(pwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    Ok((stdout, stderr))
}

/// Test that `hurry cargo build --help` produces the same output as `cargo build --help`.
#[tokio::test]
async fn build_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "build", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["build", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo build -h` produces the same output as `cargo build -h`.
#[tokio::test]
async fn build_short_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "build", "-h"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["build", "-h"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo help build` produces the same output as `cargo help build`.
#[tokio::test]
async fn help_build() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "help", "build"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["help", "build"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo check --help` produces the same output as `cargo check --help`.
#[tokio::test]
async fn check_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "check", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["check", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo check -h` produces the same output as `cargo check -h`.
#[tokio::test]
async fn check_short_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "check", "-h"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["check", "-h"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo test --help` produces the same output as `cargo test --help`.
#[tokio::test]
async fn test_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "test", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["test", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo test -h` produces the same output as `cargo test -h`.
#[tokio::test]
async fn test_short_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "test", "-h"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["test", "-h"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo help test` produces the same output as `cargo help test`.
#[tokio::test]
async fn help_test() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "help", "test"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["help", "test"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo clean --help` produces the same output as `cargo clean --help`.
#[tokio::test]
async fn clean_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "clean", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["clean", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo doc --help` produces the same output as `cargo doc --help`.
#[tokio::test]
async fn doc_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "doc", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["doc", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo tree --help` produces the same output as `cargo tree --help`.
#[tokio::test]
async fn tree_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "tree", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["tree", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo version` produces the same output as `cargo version`.
#[tokio::test]
async fn version() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "version"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["version"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo --version` produces the same output as `cargo --version`.
#[tokio::test]
async fn version_flag() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "--version"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["--version"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo help` produces the same output as `cargo help`.
#[tokio::test]
async fn help_no_subcommand() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}

/// Test that `hurry cargo run --help` produces the same output as `cargo run --help`.
#[tokio::test]
async fn run_long_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let (hurry_stdout, hurry_stderr) = run_command("hurry", &["cargo", "run", "--help"], pwd)?;
    let (cargo_stdout, cargo_stderr) = run_command("cargo", &["run", "--help"], pwd)?;

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");

    Ok(())
}
