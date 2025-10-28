//! Tests for cargo command passthrough functionality.
//!
//! These tests verify that hurry correctly forwards cargo commands and help
//! requests to the underlying cargo binary by comparing the output of
//! `hurry cargo ...` directly with `cargo ...`.

use pretty_assertions::assert_eq as pretty_assert_eq;
use simple_test_case::test_case;
use std::process::{Command, Stdio};

/// Run a command and capture its output (both stdout and stderr).
fn run_command(name: &str, args: &[&str]) -> std::io::Result<(String, String)> {
    let output = Command::new(name)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((stdout, stderr))
}

/// Run hurry-dev with the given arguments.
#[track_caller]
fn run_hurry(args: &[&str]) -> (String, String) {
    match run_command("hurry-dev", args) {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            panic!("run `make install-dev` to install hurry-dev")
        }
        Err(e) => panic!("failed to execute 'hurry-dev': {e}"),
    }
}

/// Run cargo with the given arguments.
#[track_caller]
fn run_cargo(args: &[&str]) -> (String, String) {
    run_command("cargo", args).unwrap_or_else(|e| panic!("failed to execute 'cargo': {e}"))
}

/// Compare hurry output with cargo output for given args.
#[track_caller]
fn assert_passthrough(args: &[&str]) {
    let mut hurry_args = vec!["cargo"];
    hurry_args.extend_from_slice(args);

    let (hurry_stdout, hurry_stderr) = run_hurry(&hurry_args);
    let (cargo_stdout, cargo_stderr) = run_cargo(args);

    pretty_assert_eq!(hurry_stdout, cargo_stdout, "stdout should match");
    pretty_assert_eq!(hurry_stderr, cargo_stderr, "stderr should match");
}

#[test_case(&["--version"]; "version_flag")]
#[test_case(&["-V"]; "version_short")]
#[test_case(&["version"]; "version_command")]
#[test_case(&["--help"]; "help_flag")]
#[test_case(&["-h"]; "help_short")]
#[test_case(&["help"]; "help_command")]
#[test_case(&["--list"]; "list_flag")]
#[test]
fn top_level_commands(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["build", "--help"]; "long_help")]
#[test_case(&["build", "-h"]; "short_help")]
#[test_case(&["help", "build"]; "help_command")]
#[test_case(&["b", "--help"]; "alias_help")]
#[test]
fn build_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["check", "--help"]; "long_help")]
#[test_case(&["check", "-h"]; "short_help")]
#[test_case(&["help", "check"]; "help_command")]
#[test_case(&["c", "--help"]; "alias_help")]
#[test]
fn check_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["test", "--help"]; "long_help")]
#[test_case(&["test", "-h"]; "short_help")]
#[test_case(&["help", "test"]; "help_command")]
#[test_case(&["t", "--help"]; "alias_help")]
#[test]
fn test_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["run", "--help"]; "long_help")]
#[test_case(&["run", "-h"]; "short_help")]
#[test_case(&["help", "run"]; "help_command")]
#[test_case(&["r", "--help"]; "alias_help")]
#[test]
fn run_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["doc", "--help"]; "long_help")]
#[test_case(&["doc", "-h"]; "short_help")]
#[test_case(&["help", "doc"]; "help_command")]
#[test_case(&["d", "--help"]; "alias_help")]
#[test]
fn doc_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["clean", "--help"]; "long_help")]
#[test_case(&["clean", "-h"]; "short_help")]
#[test_case(&["help", "clean"]; "help_command")]
#[test]
fn clean_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["bench", "--help"]; "long_help")]
#[test_case(&["bench", "-h"]; "short_help")]
#[test_case(&["help", "bench"]; "help_command")]
#[test]
fn bench_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["update", "--help"]; "long_help")]
#[test_case(&["update", "-h"]; "short_help")]
#[test_case(&["help", "update"]; "help_command")]
#[test]
fn update_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["search", "--help"]; "long_help")]
#[test_case(&["search", "-h"]; "short_help")]
#[test_case(&["help", "search"]; "help_command")]
#[test]
fn search_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["publish", "--help"]; "long_help")]
#[test_case(&["publish", "-h"]; "short_help")]
#[test_case(&["help", "publish"]; "help_command")]
#[test]
fn publish_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["install", "--help"]; "long_help")]
#[test_case(&["install", "-h"]; "short_help")]
#[test_case(&["help", "install"]; "help_command")]
#[test]
fn install_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["uninstall", "--help"]; "long_help")]
#[test_case(&["uninstall", "-h"]; "short_help")]
#[test_case(&["help", "uninstall"]; "help_command")]
#[test]
fn uninstall_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["new", "--help"]; "long_help")]
#[test_case(&["new", "-h"]; "short_help")]
#[test_case(&["help", "new"]; "help_command")]
#[test]
fn new_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["init", "--help"]; "long_help")]
#[test_case(&["init", "-h"]; "short_help")]
#[test_case(&["help", "init"]; "help_command")]
#[test]
fn init_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["add", "--help"]; "long_help")]
#[test_case(&["add", "-h"]; "short_help")]
#[test_case(&["help", "add"]; "help_command")]
#[test]
fn add_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["remove", "--help"]; "long_help")]
#[test_case(&["remove", "-h"]; "short_help")]
#[test_case(&["help", "remove"]; "help_command")]
#[test_case(&["rm", "--help"]; "alias_help")]
#[test]
fn remove_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["tree", "--help"]; "long_help")]
#[test_case(&["tree", "-h"]; "short_help")]
#[test_case(&["help", "tree"]; "help_command")]
#[test]
fn tree_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["metadata", "--help"]; "long_help")]
#[test_case(&["metadata", "-h"]; "short_help")]
#[test_case(&["help", "metadata"]; "help_command")]
#[test]
fn metadata_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["fetch", "--help"]; "long_help")]
#[test_case(&["fetch", "-h"]; "short_help")]
#[test_case(&["help", "fetch"]; "help_command")]
#[test]
fn fetch_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["fix", "--help"]; "long_help")]
#[test_case(&["fix", "-h"]; "short_help")]
#[test_case(&["help", "fix"]; "help_command")]
#[test]
fn fix_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["rustc", "--help"]; "long_help")]
#[test_case(&["rustc", "-h"]; "short_help")]
#[test_case(&["help", "rustc"]; "help_command")]
#[test]
fn rustc_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["rustdoc", "--help"]; "long_help")]
#[test_case(&["rustdoc", "-h"]; "short_help")]
#[test_case(&["help", "rustdoc"]; "help_command")]
#[test]
fn rustdoc_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["package", "--help"]; "long_help")]
#[test_case(&["package", "-h"]; "short_help")]
#[test_case(&["help", "package"]; "help_command")]
#[test]
fn package_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["vendor", "--help"]; "long_help")]
#[test_case(&["vendor", "-h"]; "short_help")]
#[test_case(&["help", "vendor"]; "help_command")]
#[test]
fn vendor_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["login", "--help"]; "long_help")]
#[test_case(&["login", "-h"]; "short_help")]
#[test_case(&["help", "login"]; "help_command")]
#[test]
fn login_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["logout", "--help"]; "long_help")]
#[test_case(&["logout", "-h"]; "short_help")]
#[test_case(&["help", "logout"]; "help_command")]
#[test]
fn logout_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["owner", "--help"]; "long_help")]
#[test_case(&["owner", "-h"]; "short_help")]
#[test_case(&["help", "owner"]; "help_command")]
#[test]
fn owner_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["yank", "--help"]; "long_help")]
#[test_case(&["yank", "-h"]; "short_help")]
#[test_case(&["help", "yank"]; "help_command")]
#[test]
fn yank_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["pkgid", "--help"]; "long_help")]
#[test_case(&["pkgid", "-h"]; "short_help")]
#[test_case(&["help", "pkgid"]; "help_command")]
#[test]
fn pkgid_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["locate-project", "--help"]; "long_help")]
#[test_case(&["locate-project", "-h"]; "short_help")]
#[test_case(&["help", "locate-project"]; "help_command")]
#[test]
fn locate_project_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["generate-lockfile", "--help"]; "long_help")]
#[test_case(&["generate-lockfile", "-h"]; "short_help")]
#[test_case(&["help", "generate-lockfile"]; "help_command")]
#[test]
fn generate_lockfile_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["config", "--help"]; "long_help")]
#[test_case(&["config", "-h"]; "short_help")]
#[test_case(&["help", "config"]; "help_command")]
#[test]
fn config_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["report", "--help"]; "long_help")]
#[test_case(&["report", "-h"]; "short_help")]
#[test_case(&["help", "report"]; "help_command")]
#[test]
fn report_help(args: &[&str]) {
    assert_passthrough(args);
}

#[test_case(&["info", "--help"]; "long_help")]
#[test_case(&["info", "-h"]; "short_help")]
#[test_case(&["help", "info"]; "help_command")]
#[test]
fn info_help(args: &[&str]) {
    assert_passthrough(args);
}
