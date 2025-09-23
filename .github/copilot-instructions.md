# Copilot Instructions for hurry

## Project Overview
- **hurry** is a Rust-based tool for accelerating Cargo builds, with a focus on developer productivity and cache efficiency.
- The workspace is organized into multiple packages, notably `packages/hurry` (core logic) and `packages/e2e` (end-to-end tests).
- Builds are invoked via `hurry cargo <command>`, replacing standard Cargo workflows for speed.

## Architecture & Key Components
- **Core logic**: `packages/hurry/src/` contains modules for caching, filesystem operations, hashing, and Cargo command orchestration.
- **End-to-end tests**: `packages/e2e/tests/it/` holds integration tests simulating real-world usage and third-party scenarios.
- **Benchmarks**: `packages/hurry/benches/` provides performance tests for cache and file operations.
- **Scripts**: `scripts/` includes shell utilities for diffing and readiness checks, often used in CI or debugging.
- **Static files**: `static/cargo/` contains cache markers and metadata for build artifact management.

## Developer Workflows
- **Build**: Use `hurry cargo build` instead of `cargo build` for all local builds.
- **Test**: Run integration tests from `packages/e2e` using standard Cargo commands, e.g. `cargo test --package e2e`.
- **Benchmarks**: Execute with `cargo bench --package hurry`.
- **Debugging**: Scripts in `scripts/` (e.g., `diff-mtime.sh`, `ready.sh`) are used for troubleshooting cache and build issues.
- **Installation**: Use the provided `install.sh` for setup on macOS/Linux; see README for options.

## Project-Specific Conventions
- **Module structure**: Each package uses a flat module layout; submodules are grouped by domain (e.g., `cache/`, `cargo/` in `src/`).
- **Testing**: End-to-end tests are placed in `tests/it/` and may interact with local/third-party binaries.
- **Cache management**: Relies on marker files in `static/cargo/` and custom logic in `src/cache.rs`.
- **No Windows support**: All scripts and workflows assume Unix-like environments.

## Integration Points & Dependencies
- **Cargo**: All build/test flows wrap or invoke Cargo commands.
- **External scripts**: Shell scripts in `scripts/` are used for CI and local debugging.
- **Release binaries**: Manual installation is supported via GitHub releases.

## Examples
- To build: `hurry cargo build`
- To run e2e tests: `cargo test --package e2e`
- To debug cache: `bash scripts/diff-mtime.sh`

## Debugging & Cache Management
- **Cache management**: Use `hurry cache reset [--yes]` to reset the user cache.
- **Debugging**: Use `hurry debug metadata <DIR>` to enumerate tracked file metadata, or `hurry debug copy <SRC> <DEST>` to copy directories recursively.
- **Metadata diffing**: Combine `hurry debug metadata` with tools like `riff` to compare metadata between folders (e.g., `hurry debug metadata folder1 | riff - hurry debug metadata folder2`).
- **Scripts**: Continue to use shell scripts in `scripts/` for advanced troubleshooting.

## Key Files & Directories
- `packages/hurry/src/` — core implementation
- `packages/e2e/tests/it/` — integration tests
- `scripts/` — developer utilities
- `static/cargo/` — cache metadata
- `install.sh` — installation script
- `README.md` — usage and setup instructions

---

**Feedback requested:** If any conventions, workflows, or integration points are unclear or missing, please specify so this guide can be improved for future AI agents.
