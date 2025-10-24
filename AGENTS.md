# Repository Guidelines

This file provides guidance for AI coding assistants (Claude Code, OpenAI Codex, Windsurf, Cursor, Zed, etc) when working with code in this repository.

## Project Overview

This is a monorepo containing two main projects:

**hurry** is a Rust tool that accelerates Cargo builds by intelligently caching and restoring build artifacts across git branches, worktrees, and development contexts. It provides drop-in replacements for common Cargo commands with significant performance improvements.

**courier** is the API service for Hurry, providing content-addressed storage (CAS) functionality. It's a simple web service built with Axum that handles blob storage with zstd compression, optimized for on-premise deployments where authentication is not a concern. PostgreSQL is available for future distributed caching features.

## Workspace Structure

- `packages/hurry/`: Core hurry implementation with CLI (`src/bin/hurry/main.rs` and command implementations in `src/bin/hurry/cmd/`), caching (`src/cache/`), cargo integration (`src/cargo/`), filesystem operations (`src/fs.rs`), hashing (`src/hash.rs`), and CAS client (`src/cas.rs`)
- `packages/courier/`: API service with API routes (`src/api/`), database (`src/db.rs`), and storage (`src/storage.rs`)
- `packages/e2e/`: End-to-end integration tests package that simulates real-world usage scenarios across git operations, branch switches, and cache restore workflows
- `static/cargo/`: Contains cache markers and metadata for build artifact management
- `scripts/`: Debugging and validation scripts
- `target/`: Build output (do not commit)

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

## Development Commands

### Setup
- Copy environment file: `cp .env.example .env` and customize as needed
  - `COURIER_DATABASE_URL`: PostgreSQL connection string for courier
  - `CAS_ROOT`: Directory path for content-addressed storage

### Building and Testing
- **Build the project**: `cargo build` for local development
- **Install hurry locally**: `cargo install --path ./packages/hurry --locked`
- **Run tests for a package**: `cargo nextest run -p {PACKAGE_NAME}`
- **Run benchmarks**: `cargo bench --package hurry`
- **Makefile shortcuts**: Common development commands are available via `make`:
  - `make dev` / `make release`: Compile debug or optimized workspaces
  - `make format`: Format code (uses nightly rustfmt)
  - `make check`: Run clippy linter
  - `make check-fix`: Run clippy with automatic fixes
  - `make sqlx-prepare`: Prepare sqlx metadata after making changes to SQL queries or database schemas
  - `make precommit`: Run all pre-commit checks and tasks

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

#### Release Management
- `scripts/release.sh`: Automated release script for publishing to S3
  - Usage: `./scripts/release.sh <version>` (e.g., `./scripts/release.sh 1.0.0`)
  - Supports options: `--dry-run`, `--skip-build`, `--skip-upload`
  - Automatically tags git, builds for all platforms, generates checksums, and uploads to S3
  - Run `aws sso login --profile <your-profile-name>` first to authenticate
  - After release, push the git tag: `git push origin v<version>`

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
- **Prepare sqlx metadata**: `make sqlx-prepare` after making changes to SQL queries or database schemas in the codebase
- **Note**: Migrations are not auto-applied on server startup to prevent accidental production migrations
- **Note**: When using sqlx-cli commands, you must manually specify `--database-url "$COURIER_DATABASE_URL"` since sqlx doesn't support per-package database URLs
- **Note**: sqlx metadata files are now stored per-package in `packages/{package}/.sqlx/` rather than at the workspace root

#### Testing
- **Run API tests**: `RUST_BACKTRACE=1 cargo test --package courier` or `cargo nextest run -p courier`
- Tests automatically spin up isolated test servers with temporary storage and database pools

## Development Workflow

### Hurry Workflow
1. Use `hurry cargo build` for all local builds instead of `cargo build`
2. Use `scripts/ready.sh` to set up a clean testing environment
3. Use the diff scripts to validate cache correctness when making changes
4. Run e2e tests to ensure integration works across different scenarios

### Courier Workflow
1. Set up environment: `cp .env.example .env` and customize as needed
2. Start PostgreSQL: `docker compose up -d postgres` (or use full `docker compose up` for everything)
3. Apply migrations: `cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url "$COURIER_DATABASE_URL"` (or `docker compose run --build migrate`)
4. Run the server: `courier serve` (reads `COURIER_DATABASE_URL` from `.env` via build.rs)
5. Make API requests: Use curl, xh, httpie, or the test client
6. Iterate on code: Tests use isolated databases via `#[sqlx::test]` macro
7. Schema changes: Edit `schema/schema.sql` → run `sql-schema migration --name {name}` → review migrations → apply with sqlx-cli → run `make sqlx-prepare`

### Release Workflow

Hurry uses S3-based distribution for binary releases. Releases are created manually using a local script.

#### Prerequisites
- AWS SSO access configured with appropriate S3 permissions
- `cross` installed for cross-compilation: `cargo install cross`
- `cargo-set-version` installed: `cargo install cargo-set-version`
- `jq` or `jaq` for JSON processing

#### Release Process

1. **Authenticate with AWS**:
   ```bash
   aws sso login --profile <your-profile-name>
   ```

2. **Create and test release** (dry run recommended first):
   ```bash
   # Dry run: builds everything but doesn't upload or create tags
   ./scripts/release.sh 1.0.0 --dry-run

   # Review artifacts in target/release-artifacts/
   ```

3. **Publish release**:
   ```bash
   # For stable releases
   ./scripts/release.sh 1.0.0

   # For prereleases (won't update /latest/ pointer)
   ./scripts/release.sh 1.0.0-beta.1
   ```

4. **Push git tag**:
   ```bash
   git push origin v1.0.0
   ```

5. **Verify release**:
   ```bash
   # Check S3 structure
   aws s3 ls s3://hurry-releases/releases/ --recursive --profile <your-profile-name>

   # Test installer
   curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash -s -- -v 1.0.0
   ```

#### Release Artifacts

The release script builds and uploads:
- Binaries for 6 platforms (macOS x86_64/arm64, Linux glibc/musl x86_64/arm64)
- SHA256 checksums file
- `versions.json` manifest with release metadata

S3 structure:
```
s3://hurry-releases/releases/
├── v1.0.0/                    # Versioned releases (immutable)
│   ├── hurry-*.tar.gz         # Platform-specific archives
│   └── checksums.txt
├── latest/                    # Points to latest stable release
│   └── hurry-*.tar.gz
└── versions.json              # Machine-readable version list
```

#### Release Script Options

- `--dry-run`: Build and test without uploading or creating git tags
- `--skip-build`: Use existing artifacts from `target/release-artifacts/`
- `--skip-upload`: Build but don't upload to S3 (useful for testing builds)

#### Important Notes

- Version changes in `Cargo.toml` are temporary and not committed
- Prereleases (e.g., `1.0.0-beta.1`) don't update the `/latest/` pointer
- Cache headers are set appropriately: versioned releases are immutable, latest is no-cache
- The script uses `cross` for Linux targets when building from macOS

## Rust Code Style

### String Creation
- Use `String::from("...")` instead of `"...".to_string()`
- Use `String::new()` instead of `"".to_string()`

### Type Annotations
- Always use postfix types (turbofish syntax)
- ❌ `let foo: Vec<_> = items.collect()`
- ✅ `let foo = items.collect::<Vec<_>>()`
- ✅ `let foo = items.collect_vec()` (with itertools)

### Control Flow
Prefer `let Some(value) = option else { ... }` over checking `.is_none()` and using `.unwrap()`:
```rust
// ❌ Avoid
if value.is_none() {
    return handle_none();
}
let value = value.unwrap();

// ✅ Prefer
let Some(value) = value else {
    return handle_none();
};
```

This makes the control flow explicit and immune to bugs from forgetting to handle the None case.

### Naming Conventions

#### Type Names: Avoid Stuttering
When a type is already namespaced by its module, don't repeat context in the type name.

- ❌ `storage::CasStorage` — stutters "storage"
- ✅ `storage::Disk` — describes implementation

- ❌ `db::Database` — generic, stutters "db"
- ✅ `db::Postgres` — specific implementation

- ❌ `cache::KeyCache` — stutters "cache"
- ✅ `cache::Memory` — describes mechanism

- ❌ `auth::JwtManager` — "manager" adds no value
- ✅ `auth::Jwt` — concise, module provides context

#### Enum Variant Names
For enums with multiple variants for the same logical concept, use a single canonical variant:
```rust
// ❌ Avoid separate variants for aliases
enum Arg {
    Package(String),
    PackageShort(String),  // -p
}

// ✅ Use alias function to normalize to canonical form
enum Arg {
    Package(String),  // both --package and -p parse to this
}

fn alias(s: &str) -> &str {
    match s {
        "-p" => "--package",
        _ => s,
    }
}
```

Benefits:
- Single representation ensures consistent behavior
- Simpler pattern matching (no need to handle multiple variants)
- Clear canonical form for serialization/rendering

#### Function Names
- Don't prefix test functions with `test_` (avoid stuttering)
- ❌ `fn test_parses_config()`
- ✅ `fn parses_config()`

### Variable Names
- Don't use hungarian notation; prefer just shadowing.
- ❌: `formats_str`
- ✅: `formats`

### Import Style
Prefer direct imports over fully qualified paths unless ambiguous:

```rust
// ✅ Prefer: Import and use directly
use client::courier::v1::{Key, cache::ArtifactFile};

let key = Key::from_hex(&hex_string)?;
let artifact = ArtifactFile::builder()
    .object_key(key)
    .build();

// ❌ Avoid: Fully qualified paths when unambiguous
let key = client::courier::v1::Key::from_hex(&hex_string)?;
let artifact = client::courier::v1::cache::ArtifactFile::builder()
    .object_key(key)
    .build();
```

**Exceptions where fully qualified paths are preferred:**
- When the function/type name is ambiguous or unclear on its own (e.g., `serde_json::to_string` is clearer than a freestanding `to_string`)
- When multiple types with the same name exist in scope (use one-level-up imports or aliases)
- When the import would create naming conflicts

### String Formatting
- Always inline rust variables in format-like strings if they can be inlined:
- Plain variables can be inlined: `format!("Hello, {name}")`
- Expressions cannot be inlined: `format!("Hello, {}", user.name())`

### Documentation
- Don't bold bullet points in markdown
- ❌ `- **Hook**: message`
- ✅ `- Hook: message`
- Avoid the "space dash space" pattern when writing prose/comments, use ":" instead.
- ❌: "All commands work the same way - do x then y"
- ✅: "All commands work the same way: do x then y"

### Other Preferences
- Use `if Some(...) = ... else { ... }` or `let ... = match { ... }` pattern over `is_some` or `is_ok` usage.
- Do not use `mod.rs`. Always prefer to create Rust modules using a `.rs` file, then put other files inside a directory with the same name.
  - ✅ Good: `my_module.rs`, `my_module/other_file.rs`
  - ❌ Bad: `my_module/mod.rs`, `my_module/other_file.rs`
- When adding packages to `Cargo.toml`, use `cargo add` instead of adding the package manually. After adding all packages, run `cargo autoinherit` to update workspace dependencies.
- After writing a batch of Rust changes, use `make format` to format code.
- After writing a batch of Rust changes, run `cargo clippy` on the project.

### Error Handling
- Use `color-eyre` for error handling and reporting
- Only panic if the problem is an invariant violation that makes it impossible for the program to continue safely, or in test code
- Prefer returning `Result` types for recoverable errors

## Testing Strategy

### General Testing Principles
- Tests are colocated with code: Tests are written in `#[cfg(test)]` modules within source files, not in separate `tests/` directories
- Integration-style tests: Even though tests are colocated, write them integration-style (testing through public APIs) rather than unit-style (testing internal implementation details)
- Running tests: Use `cargo nextest run -p {PACKAGE_NAME}` to run tests for a specific package

### Assertions
Use `pretty_assertions` with prefixed imports:
```rust
use pretty_assertions::{
    assert_eq as pretty_assert_eq,
    assert_ne as pretty_assert_ne,
    assert_matches as pretty_assert_matches,
};
```

Always construct the ENTIRE expected value upfront and compare in ONE operation:
```rust
// ✅ Prefer: Declare expected value first, single assertion
let expected = serde_json::json!({
    "written": [key1, key2, key3],
    "skipped": [],
    "errors": [],
});
let body = response.json::<Value>();
pretty_assert_eq!(body, expected);

// ❌ Avoid: Property-by-property assertions
let body = response.json::<Value>();
pretty_assert_eq!(body["written"].len(), 3);
pretty_assert_eq!(body["skipped"], serde_json::json!([]));
assert!(body["written"].contains(&key1));

// ❌ Avoid: Using matches! when you can construct the full value
assert!(matches!(args.0[0], CargoBuildArgument::GenericFlag(ref flag) if flag == "--flag"));

// ✅ Prefer: Construct full expected value
let expected = vec![CargoBuildArgument::GenericFlag(String::from("--flag"))];
pretty_assert_eq!(args.0, expected);
```

For non-deterministic values (like error messages), keep property checks minimal and don't copy values from response bodies:
```rust
// ✅ Good: Check structure separately for unpredictable values
pretty_assert_eq!(body["written"], serde_json::json!([]));
pretty_assert_eq!(body["errors"].as_array().unwrap().len(), 1);
assert!(body["errors"][0]["error"].as_str().unwrap().contains("expected substring"));

// ❌ Bad: Copying from response body
let expected = serde_json::json!({
    "errors": [body["errors"][0].clone()],  // Don't do this!
});
```

### Parameterized Tests
Use `simple_test_case` for tests with multiple variations:
```rust
use simple_test_case::test_case;

#[test_case("--flag"; "long")]
#[test_case("-f"; "short")]
#[test]
fn parses_flag(flag: &str) {
    let args = parse(vec![flag]);
    let expected = vec![Flag];
    pretty_assert_eq!(args, expected);
}
```

Benefits:
- Each test case runs independently with clear naming (e.g., `parses_flag::long`, `parses_flag::short`)
- Test data is not monotonically increasing (use distinct names like `foo`, `bar`, `baz` instead of `value1`, `value2`, `value3`)
- Failures show which specific case failed

### Parsing with Multiple Input Formats
When implementing parsers that accept multiple input formats, test all variations:

For flags with values:
- Long form with space: `--flag value`
- Long form with equals: `--flag=value`
- Short form with space: `-f value`
- Short form with equals: `-f=value`

For list/collection inputs:
- Different delimiters: comma-separated vs space-separated
- Multiple invocations: `--flag a --flag b`
- Combined: `--flag a,b --flag c`

### Hurry Testing
- End-to-end tests: Full workflow validation in `packages/e2e/`
- Manual validation: Use `scripts/diff-*.sh` to verify cache restore accuracy
- Benchmarks: Performance regression testing via `cargo bench`

### Courier Testing
- API tests: Use `#[sqlx::test]` macro for automatic database setup with migrations
- Test isolation: Each test gets its own PostgreSQL database instance and temporary storage directory
- Test helpers: Use `test_server()` to create isolated test server, `write_cas()` for storage operations

### Test Workflow
After adding tests to a file:
1. Run tests for the package: `cargo nextest run -p {PACKAGE_NAME}`
2. If successful, commit the changes
3. If tests fail, fix the issues before committing

## Cache Correctness

hurry's core value proposition depends on cache correctness. When making changes:
1. Run `scripts/diff-mtime.sh` to verify mtime preservation
2. Run `scripts/diff-tree.sh` to verify directory structure consistency
3. Ensure end-to-end tests pass for various git scenarios
4. Test across different cargo profiles and dependency changes

**IMPORTANT**: Do NOT use mtime comparisons when deciding whether to restore from cache. mtimes are preserved but not used as cache invalidation criteria.

## I/O Operations
- Prefer streaming operations over buffered ones by default
- Use `AsyncRead`, `AsyncWrite`, `Read`, `Write` traits directly

## Build System Notes

- Uses Rust 2024 edition
- Workspace-based dependency management in root `Cargo.toml`
- No Windows support (Unix-only scripts and workflows)
- Heavy use of async/await patterns with tokio runtime
- Extensive use of workspace dependencies for consistency
- Courier uses `build.rs` to set `DATABASE_URL` from `COURIER_DATABASE_URL` for sqlx compatibility
- `rust-toolchain.toml` pins toolchains—avoid `rustup override` unless debugging
- Never commit secrets; keep real credentials external
- Clean `target/` and reset caches when benchmarking

## Commit & PR Guidelines

Follow the repository's conventional commit style (`feat: cache warmup (#123)`). Keep commits small, reversible, and lint-clean. PRs need a concise summary, validation or reproduction steps, and screenshots/logs when CLI output shifts. Call out migrations, new env vars, or cache-impacting changes in bold to help reviewers.

Don't commit for me unless I ask you to.
