# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a monorepo containing two main projects:

**hurry** is a Rust tool that accelerates Cargo builds by intelligently caching and restoring build artifacts across git branches, worktrees, and development contexts. It provides drop-in replacements for common Cargo commands with significant performance improvements.

**courier** is the API service for Hurry, providing content-addressed storage (CAS) functionality with authentication, compression, and access control. It's a web service built with Axum that handles blob storage, user authentication via PASETO tokens, and PostgreSQL-backed metadata management.

## Development Commands

### Prerequisites
- **Initialize git submodules**: `git submodule update --init --recursive`
  - This project vendors sqlx 0.9 (pre-release) at a specific commit for stability
  - Ensures all developers use the same sqlx version and prevents alpha changes from breaking builds
- **Install sqlx-cli**: `cargo install --path vendor/sqlx/sqlx-cli --features postgres,sqlite,sqlx-toml`
  - The `sqlx-toml` feature is required for multi-database workspace support
  - Install both `postgres` and `sqlite` features to work with both databases

### Building and Testing
- **Build the project**: `hurry cargo build` (use instead of `cargo build`)
- **Install hurry locally**: `cargo install --path ./packages/hurry --locked`
- **Run tests for a package**: `cargo nextest run -p {PACKAGE_NAME}`
- **Run benchmarks**: `cargo bench --package hurry`

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
- **Run locally**: `courier serve --database-url <URL> --cas-root <PATH>` (uses `COURIER_DATABASE_URL` from `.env`)
- **Run in Docker**: `docker compose up` (automatically applies migrations)
- **View serve options**: `courier serve --help`

#### Database Management
- **Apply migrations manually**:
  - Via sqlx-cli: Run from courier directory: `cd packages/courier && cargo sqlx migrate run` (recommended for dev, faster)
  - Via courier binary: `docker compose run --build migrate` (for testing production-like deployments)
- **Generate new migration**: `sql-schema migration --name {migration_name}` (after editing `schema/schema.sql`)
- **Run tests with fixtures**: Tests use `#[sqlx::test]` macro with automatic fixture loading
- **Note**: All sqlx-cli commands must be run from within the `packages/courier` directory to pick up the `sqlx.toml` configuration
- **Note**: Migrations are not auto-applied on server startup to prevent accidental production migrations
- **Note**: Courier uses `COURIER_DATABASE_URL` instead of `DATABASE_URL` to avoid conflicts with other packages in the workspace

#### Testing
- **Run API tests**: `cargo nextest run -p courier` (uses `COURIER_DATABASE_URL` from `.env`)
- Tests automatically spin up isolated test servers with temporary storage and database pools

## Architecture

### Workspace Structure
- `packages/hurry/`: Core hurry implementation with modules for caching (`cache/`), cargo integration (`cargo/`), filesystem operations (`fs.rs`), and hashing (`hash.rs`)
- `packages/courier/`: API service with modules for API routes (`api/`), authentication (`auth.rs`), database (`db.rs`), and storage (`storage.rs`)
- `packages/e2e/`: End-to-end integration tests that simulate real-world usage scenarios
- `static/cargo/`: Contains cache markers and metadata for build artifact management

### Hurry Components
- Cache system (`packages/hurry/src/cache/`): Manages build artifact caching across different git states
- Cargo integration (`packages/hurry/src/cargo/`): Handles workspace metadata, dependencies, and build profiles
- File operations (`packages/hurry/src/fs.rs`): Optimized filesystem operations with mtime preservation

### Courier Components
- API routes (`packages/courier/src/api/`): Versioned HTTP handlers using Axum
  - `/api/v1/auth`: Token minting and validation endpoints
  - `/api/v1/cas`: Content-addressed storage read/write/check operations
  - `/api/v1/health`: Health check endpoint
- Authentication (`packages/courier/src/auth.rs`): PASETO-based stateless tokens with per-instance secrets, LRU caching for CAS key validation
- Database (`packages/courier/src/db.rs`): PostgreSQL integration via sqlx with migrations
- Storage (`packages/courier/src/storage.rs`): Disk-based CAS with zstd compression, blake3 hashing, atomic writes
- Schema (`packages/courier/schema/`): SQL schema definitions and migration files
  - `schema.sql`: Canonical database state (hand-maintained)
  - `migrations/`: Generated up/down migrations via `sql-schema`
  - `fixtures/`: Test data fixtures

### Courier Data Model
- Organizations: Multi-tenant isolation
- Users: Belong to organizations, authenticate via API keys
- API Keys: Long-lived tokens for user authentication
- CAS Keys: Content-addressed blob identifiers (blake3 hashes)
- Access Control: Organization-level permissions for CAS keys
- Frequency Tracking: Per-user CAS key access patterns

## Development Workflow

### Hurry Workflow
1. Use `hurry cargo build` for all local builds instead of `cargo build`
2. Use `scripts/ready.sh` to set up a clean testing environment
3. Use the diff scripts to validate cache correctness when making changes
4. Run e2e tests to ensure integration works across different scenarios

### Courier Workflow
1. Set up environment: `cp example.env .env` and customize as needed
2. Start PostgreSQL: `docker compose up -d postgres` (or use full `docker compose up` for everything)
3. Apply migrations: `cd packages/courier && cargo sqlx migrate run` (or `docker compose run --build migrate`)
4. Run the server: `courier serve`
5. Make API requests: Use curl, xh, httpie, or the test client
6. Iterate on code: Tests use isolated databases via `#[sqlx::test]` macro
7. Schema changes: Edit `schema/schema.sql` → run `sql-schema migration --name {name}` → review migrations → `cd packages/courier && cargo sqlx migrate run`

### Courier Authentication Flow
1. Client presents long-lived API key via `Authorization: Bearer <key>` header
2. Client includes `x-org-id` header to specify organization context
3. Server validates key against database, returns short-lived stateless PASETO token
4. Client uses stateless token for subsequent CAS operations (read/write/check)
5. Stateless tokens are instance-specific (not valid across server restarts or different instances)

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
- API tests: Use `#[sqlx::test]` macro for automatic database setup with migrations and fixtures
- Test isolation: Each test gets its own PostgreSQL database instance and temporary storage directory
- Test helpers: Use `test_server()` to create isolated test server, `mint_token()` and `write_cas()` for auth/storage operations
- Fixtures: Test data in `schema/fixtures/*.sql` loaded automatically by test macro

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
- Vendors sqlx 0.9 (pre-release) in `vendor/sqlx` as a git submodule pinned to a specific commit for stability and multi-database support via `sqlx.toml` configuration files

### Multi-Database Configuration

This workspace uses multiple databases:
- `courier`: PostgreSQL database (`COURIER_DATABASE_URL`)
- `hurry`: SQLite database (`HURRY_DATABASE_URL`)

Each package has its own `sqlx.toml` configuration file that specifies which environment variable to use for its database connection. This allows both packages to coexist in the same workspace without conflicting over the `DATABASE_URL` environment variable.

**Setup:**
1. Copy the example environment file: `cp example.env .env`
2. Customize database URLs in `.env` as needed

**Running Migrations:**

sqlx-cli discovers the `sqlx.toml` configuration in the current directory, so you must `cd` into each package directory:

```bash
# Courier migrations
cd packages/courier
cargo sqlx migrate run

# Hurry migrations
cd packages/hurry
cargo sqlx migrate run
```

All sqlx-cli commands (e.g., `sqlx database create`, `sqlx migrate add`, `sqlx prepare`) work the same way: run them from within the package directory.

**Running Tests:**
Tests automatically use the correct database URL from `.env` based on each package's `sqlx.toml` configuration:
```bash
cargo nextest run -p courier
cargo nextest run -p hurry
```

This is accomplished with a build script workaround for now due to `#[sqlx::test]` not yet implementing support for `sqlx.toml`.

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
