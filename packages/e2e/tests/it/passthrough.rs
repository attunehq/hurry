//! Tests for cargo command passthrough functionality.
//!
//! These tests verify that hurry correctly forwards cargo commands and help
//! requests to the underlying cargo binary, enabling users to alias
//! `hurry cargo` as their default `cargo` command.

use color_eyre::Result;
use e2e::{Command, temporary_directory};

/// Test that `hurry cargo build --help` shows cargo's help, not hurry's.
#[tokio::test]
async fn build_help_shows_cargo_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "build", "--help"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    // Verify it shows cargo's help (not hurry's hurry-specific flags)
    assert!(
        stdout.contains("Compile a local package"),
        "Should show cargo's build help"
    );
    assert!(
        !stdout.contains("hurry-courier-url"),
        "Should not show hurry-specific flags"
    );
    assert!(
        stdout.contains("--message-format"),
        "Should show standard cargo build flags"
    );

    Ok(())
}

/// Test that `hurry cargo build -h` shows cargo's short help.
#[tokio::test]
async fn build_short_help_shows_cargo_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "build", "-h"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    assert!(
        stdout.contains("Compile a local package"),
        "Should show cargo's build help with -h flag"
    );
    assert!(
        !stdout.contains("hurry-courier-url"),
        "Should not show hurry-specific flags with -h"
    );

    Ok(())
}

/// Test that `hurry cargo help build` shows cargo's man page help.
#[tokio::test]
async fn help_build_shows_cargo_help() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "help", "build"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    // cargo help shows man page format
    assert!(
        stdout.contains("cargo-build") || stdout.contains("Compile"),
        "Should show cargo's man page help for build"
    );

    Ok(())
}

/// Test that passthrough commands like `check` work correctly.
#[tokio::test]
async fn check_help_passthrough() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "check", "--help"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    assert!(
        stdout.contains("Check a local package"),
        "Should show cargo check help"
    );
    assert!(
        stdout.contains("--message-format"),
        "Should show cargo check flags"
    );

    Ok(())
}

/// Test that `hurry cargo test --help` shows cargo's test help.
#[tokio::test]
async fn test_help_passthrough() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "test", "--help"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    assert!(
        stdout.contains("Execute all unit and integration tests"),
        "Should show cargo test help"
    );
    assert!(
        stdout.contains("--no-run"),
        "Should show test-specific flags"
    );

    Ok(())
}

/// Test that `hurry cargo help test` shows cargo's test man page.
#[tokio::test]
async fn help_test_passthrough() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "help", "test"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    assert!(
        stdout.contains("cargo-test") || stdout.contains("Execute"),
        "Should show cargo's man page help for test"
    );

    Ok(())
}

/// Test that external subcommands (plugins) work through passthrough.
#[tokio::test]
async fn external_command_version() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    let output = Command::new()
        .pwd(pwd)
        .name("hurry")
        .args(["cargo", "version"])
        .finish()
        .run_local_with_output()?;

    let stdout = output.stdout_lossy_string();

    assert!(
        stdout.contains("cargo"),
        "Should show cargo version"
    );

    Ok(())
}

/// Test that standard cargo commands execute correctly through passthrough.
#[tokio::test]
async fn passthrough_command_execution() -> Result<()> {
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    // Create a simple Rust project
    Command::new()
        .pwd(pwd)
        .name("cargo")
        .args(["new", "test-passthrough"])
        .finish()
        .run_local()?;

    let project_dir = pwd.join("test-passthrough");

    // Test that hurry cargo check works
    Command::new()
        .pwd(&project_dir)
        .name("hurry")
        .args(["cargo", "check"])
        .finish()
        .run_local()?;

    // Test that hurry cargo clean works
    Command::new()
        .pwd(&project_dir)
        .name("hurry")
        .args(["cargo", "clean"])
        .finish()
        .run_local()?;

    Ok(())
}

/// Test that `hurry cargo` with no subcommand shows hurry's command list.
#[tokio::test]
async fn no_subcommand_shows_hurry_help() -> Result<()> {
    use std::process::Stdio;
    let _ = color_eyre::install()?;

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();

    // Run command and capture output even if it fails (clap help exits with error)
    let output = std::process::Command::new("hurry")
        .current_dir(pwd)
        .args(["cargo"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    // Should show hurry's command list (could be in stdout or stderr)
    assert!(
        combined.contains("Fast `cargo` builds") || combined.contains("Usage: hurry cargo"),
        "Should show hurry's cargo subcommand help"
    );
    assert!(
        combined.contains("build") && combined.contains("check"),
        "Should list available commands"
    );

    Ok(())
}
