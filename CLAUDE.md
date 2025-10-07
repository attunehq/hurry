# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**hurry** is a Rust tool that accelerates Cargo builds by intelligently caching and restoring build artifacts across git branches, worktrees, and development contexts. It provides drop-in replacements for common Cargo commands with significant performance improvements.

## Development Commands

### Building and Testing
- **Build the project**: `hurry cargo build` (use instead of `cargo build`)
- **Install locally**: `cargo install --path ./packages/hurry --locked`
- **Run tests for a package**: `cargo nextest run -p {PACKAGE_NAME}`
- **Run benchmarks**: `cargo bench --package hurry`

### Cache Management
- **Reset user cache**: `hurry cache reset --yes`
- **View cache debug info**: `hurry debug metadata <directory>`
- **Copy directories with metadata**: `hurry debug copy <src> <dest>`

### Debugging Scripts
The `scripts/` directory contains specialized debugging tools:
- **`scripts/ready.sh`**: Install hurry, reset caches, and warm the cache for testing
- **`scripts/diff-mtime.sh`**: Compare restored hurry cache with cargo cache using mtime diffs
- **`scripts/diff-tree.sh`**: Compare directory trees between hurry and cargo builds

These scripts are essential for cache correctness validation and performance analysis.

## Architecture

### Workspace Structure
- **`packages/hurry/`**: Core hurry implementation with modules for caching (`cache/`), cargo integration (`cargo/`), filesystem operations (`fs.rs`), and hashing (`hash.rs`)
- **`packages/e2e/`**: End-to-end integration tests that simulate real-world usage scenarios
- **`static/cargo/`**: Contains cache markers and metadata for build artifact management

### Key Components
- **Cache system** (`packages/hurry/src/cache/`): Manages build artifact caching across different git states
- **Cargo integration** (`packages/hurry/src/cargo/`): Handles workspace metadata, dependencies, and build profiles
- **File operations** (`packages/hurry/src/fs.rs`): Optimized filesystem operations with mtime preservation

## Development Workflow

1. Use `hurry cargo build` for all local builds instead of `cargo build`
2. Use `scripts/ready.sh` to set up a clean testing environment
3. Use the diff scripts to validate cache correctness when making changes
4. Run e2e tests to ensure integration works across different scenarios

## Testing Strategy

- **Tests are colocated with code**: Tests are written in `#[cfg(test)]` modules within source files, not in separate `tests/` directories
- **Integration-style tests**: Even though tests are colocated, write them integration-style (testing through public APIs) rather than unit-style (testing internal implementation details)
- **End-to-end tests**: Full workflow validation in `packages/e2e/`
- **Manual validation**: Use `scripts/diff-*.sh` to verify cache restore accuracy
- **Benchmarks**: Performance regression testing via `cargo bench`
- **Running tests**: Use `cargo nextest run -p {PACKAGE_NAME}` to run tests for a specific package

## Cache Correctness

hurry's core value proposition depends on cache correctness. When making changes:
1. Run `scripts/diff-mtime.sh` to verify mtime preservation
2. Run `scripts/diff-tree.sh` to verify directory structure consistency
3. Ensure end-to-end tests pass for various git scenarios
4. Test across different cargo profiles and dependency changes

## Build System Notes

- Uses Rust 2024 edition
- Workspace-based dependency management in root `Cargo.toml`
- No Windows support (Unix-only scripts and workflows)
- Heavy use of async/await patterns with tokio runtime
- Extensive use of workspace dependencies for consistency

## Rust Naming Conventions

### Avoid Stuttering in Type Names

When a type is already namespaced by its module, don't repeat context in the type name. The fully-qualified path should read naturally without redundancy.

Examples:
- ❌ `storage::CasStorage` (stutters "storage")
- ✅ `storage::Disk` (clear what it does, doesn't repeat)

- ❌ `db::Database` (generic, stutters "db")
- ✅ `db::Postgres` (specific implementation, doesn't stutter)

- ❌ `cache::KeyCache` (stutters "cache")
- ✅ `cache::Memory` (describes the storage mechanism)

- ❌ `auth::JwtManager` (verbose, "manager" adds no value)
- ✅ `auth::Jwt` (concise, module provides context)

The module namespace already tells you the domain - the type name should add new information about the specific implementation or purpose.

## Additional Guidelines

- Prefer to write tests as "cargo unit tests": colocated with code in `#[cfg(test)]` modules. Write these tests integration-style over unit-style.
- Prefer streaming IO operations (e.g. AsyncRead, AsyncWrite, Read, Write) over buffered operations by default
- Prefer `pretty_assertions` over standard assertions; import them with a `pretty_` prefix:
  - `pretty_assertions::assert_eq as pretty_assert_eq`
  - `pretty_assertions::assert_ne as pretty_assert_ne`
  - `pretty_assertions::assert_matches as pretty_assert_matches`