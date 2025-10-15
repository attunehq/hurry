# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a monorepo containing two main projects:

**hurry** is a Rust tool that accelerates Cargo builds by intelligently caching and restoring build artifacts across git branches, worktrees, and development contexts. It provides drop-in replacements for common Cargo commands with significant performance improvements.

**courier** is the API service for Hurry, providing content-addressed storage (CAS) functionality. It's a simple web service built with Axum that handles blob storage with zstd compression, optimized for on-premise deployments where authentication is not a concern. PostgreSQL is available for future distributed caching features.

## Development Commands

### Setup
- Copy environment file: `cp example.env .env` and customize as needed
  - `COURIER_DATABASE_URL`: PostgreSQL connection string for courier
  - `CAS_ROOT`: Directory path for content-addressed storage

### Building and Testing
- **Build the project**: `hurry cargo build` (use instead of `cargo build`)
- **Install hurry locally**: `cargo install --path ./packages/hurry --locked`
- **Run tests for a package**: `cargo nextest run -p {PACKAGE_NAME}`
- **Run benchmarks**: `cargo bench --package hurry`
- **Makefile shortcuts**: Common development commands are available via `make`:
  - `make format`: Format code with cargo +nightly fmt
  - `make check`: Run clippy linter
  - `make check-fix`: Run clippy with automatic fixes
  - `make precommit`: Run format, check-fix, and sqlx-prepare before committing
  - `make sqlx-prepare`: Prepare sqlx metadata for courier package

### Hurry-specific Commands

#### Cache Management
- **Reset user cache**: `hurry cache reset --yes`
- **View cache debug info**: `hurry debug metadata <directory>`
- **Copy directories with metadata**: `hurry debug copy <src> <dest>`

#### Debugging Scripts
The `scripts/` directory contains specialized debugging tools:
- `scripts/ready.sh`: Install hurry, reset caches, and warm the cache for testing
- `scripts/diff-mtime.sh`: Compare restored hurry cache with cargo cache using mtime diffs
- `scripts/diff-tree.sh`: Compare directory trees between hurry and cargo builds

These scripts are essential for cache correctness validation and performance analysis.

### Courier-specific Commands

#### Running the Server
- **Run locally**: `courier serve --database-url <URL> --cas-root <PATH>`
- **Run in Docker**: `docker compose up` (automatically applies migrations)
- **View serve options**: `courier serve --help`

#### Database Management
- **Apply migrations manually**:
  - Via sqlx-cli: `cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url "$COURIER_DATABASE_URL"` (recommended for dev, faster)
  - Via courier binary: `docker compose run --build migrate` (for testing production-like deployments)
- **Generate new migration**: `sql-schema migration --name {migration_name}` (after editing `schema/schema.sql`)
- **Prepare sqlx metadata**: `make sqlx-prepare` or manually run `cd packages/courier && cargo sqlx prepare --database-url "$COURIER_DATABASE_URL"`
- **Note**: Migrations are not auto-applied on server startup to prevent accidental production migrations
- **Note**: When using sqlx-cli commands, you must manually specify `--database-url "$COURIER_DATABASE_URL"` since sqlx doesn't support per-package database URLs
- **Note**: sqlx metadata files are now stored per-package in `packages/{package}/.sqlx/` rather than at the workspace root

#### Testing
- **Run API tests**: `RUST_BACKTRACE=1 cargo test --package courier` or `cargo nextest run -p courier`
- Tests automatically spin up isolated test servers with temporary storage and database pools

## Architecture

### Workspace Structure
- `packages/hurry/`: Core hurry implementation with modules for caching (`cache/`), cargo integration (`cargo/`), filesystem operations (`fs.rs`), and hashing (`hash.rs`)
- `packages/courier/`: API service with modules for API routes (`api/`), database (`db.rs`), and storage (`storage.rs`)
- `packages/e2e/`: End-to-end integration tests that simulate real-world usage scenarios
- `static/cargo/`: Contains cache markers and metadata for build artifact management

### Hurry Components
- Cache system (`packages/hurry/src/cache/`): Manages build artifact caching across different git states
- Cargo integration (`packages/hurry/src/cargo/`): Handles workspace metadata, dependencies, and build profiles
- File operations (`packages/hurry/src/fs.rs`): Optimized filesystem operations with mtime preservation
- CAS client (`packages/hurry/src/cas.rs`): Content-addressed storage client backed by Courier

### Courier Components
- API routes (`packages/courier/src/api/`): Versioned HTTP handlers using Axum
  - `/api/v1/cas`: Content-addressed storage read/write/check operations
  - `/api/v1/cache/cargo`: Distributed cargo build cache save/restore endpoints
  - `/api/v1/health`: Health check endpoint
- Database (`packages/courier/src/db.rs`): PostgreSQL integration via sqlx with migrations for distributed caching
- Storage (`packages/courier/src/storage.rs`): Disk-based CAS with zstd compression, blake3 hashing, atomic writes
- Schema (`packages/courier/schema/`): SQL schema definitions and migration files
  - `schema.sql`: Canonical database state (hand-maintained)
  - `migrations/`: Generated up/down migrations via `sql-schema`

### Courier Data Model (Distributed Caching)
- `cargo_object`: Content-addressed storage keys (blake3 hashes)
- `cargo_package`: Package name and version pairs
- `cargo_library_unit_build`: Represents a specific build configuration with compilation unit hashes
- `cargo_library_unit_build_artifact`: Individual build artifacts with paths, mtimes, and executable flags

## Development Workflow

### Hurry Workflow
1. Use `hurry cargo build` for all local builds instead of `cargo build`
2. Use `scripts/ready.sh` to set up a clean testing environment
3. Use the diff scripts to validate cache correctness when making changes
4. Run e2e tests to ensure integration works across different scenarios

### Courier Workflow
1. Set up environment: `cp example.env .env` and customize as needed
2. Start PostgreSQL: `docker compose up -d postgres` (or use full `docker compose up` for everything)
3. Apply migrations: `cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url "$COURIER_DATABASE_URL"` (or `docker compose run --build migrate`)
4. Run the server: `courier serve` (reads `COURIER_DATABASE_URL` from `.env` via build.rs)
5. Make API requests: Use curl, xh, httpie, or the test client
6. Iterate on code: Tests use isolated databases via `#[sqlx::test]` macro
7. Schema changes: Edit `schema/schema.sql` → run `sql-schema migration --name {name}` → review migrations → apply with sqlx-cli

## Testing Strategy

### General Testing Principles
- Tests are colocated with code: Tests are written in `#[cfg(test)]` modules within source files, not in separate `tests/` directories
- Integration-style tests: Even though tests are colocated, write them integration-style (testing through public APIs) rather than unit-style (testing internal implementation details)
- Running tests: Use `cargo nextest run -p {PACKAGE_NAME}` to run tests for a specific package

### Hurry Testing
- End-to-end tests: Full workflow validation in `packages/e2e/`
- Manual validation: Use `scripts/diff-*.sh` to verify cache restore accuracy
- Benchmarks: Performance regression testing via `cargo bench`

### Courier Testing
- API tests: Use `#[sqlx::test]` macro for automatic database setup with migrations
- Test isolation: Each test gets its own PostgreSQL database instance and temporary storage directory
- Test helpers: Use `test_server()` to create isolated test server, `write_cas()` for storage operations

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
- Courier uses `build.rs` to set `DATABASE_URL` from `COURIER_DATABASE_URL` for sqlx compatibility

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
