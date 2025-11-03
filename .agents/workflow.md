# Development Workflow

## Setup
- Copy environment file: `cp .env.example .env` and customize as needed
  - `COURIER_DATABASE_URL`: PostgreSQL connection string for courier
  - `CAS_ROOT`: Directory path for content-addressed storage

## Building and Testing
- **Build the project**: `cargo build` for local development
- **Install hurry locally**: `cargo install --path ./packages/hurry --locked` (installs as `hurry`)
- **Install hurry for testing**: `make install-dev` (installs as `hurry-dev`)
  - **IMPORTANT**: When developing and testing local changes to hurry, you MUST use `hurry-dev` instead of `hurry`
  - Run `make install-dev` after making changes to propagate them to the `hurry-dev` binary
  - This avoids conflicts with your production `hurry` installation
  - Use `hurry-dev cargo build`, `hurry-dev cache reset`, etc. for testing
- **Run tests for a package**: `cargo nextest run -p {PACKAGE_NAME}`
- **Run benchmarks**: `cargo bench --package hurry`
- **Makefile shortcuts**: Common development commands are available via `make`:
  - `make dev` / `make release`: Compile debug or optimized workspaces
  - `make format`: Format code (uses nightly rustfmt)
  - `make check`: Run clippy linter
  - `make check-fix`: Run clippy with automatic fixes
  - `make sqlx-prepare`: Prepare sqlx metadata after making changes to SQL queries or database schemas
  - `make precommit`: Run all pre-commit checks and tasks
  - `make install`: Install hurry to `$CARGO_HOME/bin/hurry` (warns about conflicts)
  - `make install-dev`: Install hurry to `$CARGO_HOME/bin/hurry-dev` (recommended for development)

## Hurry-specific Commands

### Cache Management
- **Reset local cache**: `hurry cache reset --yes`
- **Reset remote cache**: `hurry cache reset --remote --yes` (deletes all cached data across entire organization)
- **View cache debug info**: `hurry debug metadata <directory>`
- **Copy directories with metadata**: `hurry debug copy <src> <dest>`

### Daemon Management
Hurry uses a background daemon for async cache uploads. The daemon starts automatically on first use.

**Daemon commands:**
- **Stop daemon**: `hurry daemon stop` (graceful shutdown with cleanup)

**Daemon debugging commands:**
- **Check daemon status**: `hurry debug daemon status` (prints "running" or "stopped")
- **View daemon context**: `hurry debug daemon context` (shows PID, URL, and log file path as JSON)
- **Extract specific field**: `hurry debug daemon context pid` (or `url`, `log_file_path`)
- **View daemon logs**: `hurry debug daemon log`
- **Follow daemon logs**: `hurry debug daemon log --follow` (like `tail -f`)

**IMPORTANT for development:**
When testing daemon changes, you MUST stop the existing daemon before running `hurry-dev`:
```bash
# Stop the old daemon
hurry-dev daemon stop

# Now run hurry-dev with your changes
hurry-dev cargo build
```

**Example workflow for debugging:**
```bash
# Check if daemon is running
hurry debug daemon status

# Get log file path and follow logs
tail -f $(hurry debug daemon context log_file_path)

# Or use built-in follow mode
hurry debug daemon log --follow

# Stop the daemon when done
hurry daemon stop
```

**Daemon context structure:**
- `pid`: Process ID of the running daemon
- `url`: HTTP endpoint the daemon is listening on (localhost)
- `log_file_path`: Absolute path to daemon log file (in user cache directory)

### Debugging Scripts
The `scripts/` directory contains specialized debugging tools:
- `scripts/ready.sh`: Install hurry, reset caches, and warm the cache for testing
- `scripts/diff-mtime.sh`: Compare restored hurry cache with cargo cache using mtime diffs
- `scripts/diff-tree.sh`: Compare directory trees between hurry and cargo builds

These scripts are essential for cache correctness validation and performance analysis.

### Release Management
- `scripts/release.sh`: Automated release script for publishing to S3
  - Usage: `./scripts/release.sh <version>` (e.g., `./scripts/release.sh 1.0.0`)
  - Supports options: `--dry-run`, `--skip-build`, `--skip-upload`
  - Automatically tags git, builds for all platforms, generates checksums, and uploads to S3
  - Run `aws sso login --profile <your-profile-name>` first to authenticate
  - After release, push the git tag: `git push origin v<version>`

## Courier-specific Commands

### Running the Server
- **Run locally**: `courier serve --database-url <URL> --cas-root <PATH>`
- **Run in Docker**: `docker compose up` (automatically applies migrations)
- **View serve options**: `courier serve --help`

### Database Management
- **Apply migrations manually**:
  - Via sqlx-cli: `cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url "$COURIER_DATABASE_URL"` (recommended for dev, faster)
  - Via courier binary: `docker compose run --build migrate` (for testing production-like deployments)
- **Generate new migration**: `sql-schema migration --name {migration_name}` (after editing `schema/schema.sql`)
- **Prepare sqlx metadata**: `make sqlx-prepare` after making changes to SQL queries or database schemas in the codebase
- **Note**: Migrations are not auto-applied on server startup to prevent accidental production migrations
- **Note**: When using sqlx-cli commands, you must manually specify `--database-url "$COURIER_DATABASE_URL"` since sqlx doesn't support per-package database URLs
- **Note**: sqlx metadata files are now stored per-package in `packages/{package}/.sqlx/` rather than at the workspace root

### Testing
- **Run API tests**: `RUST_BACKTRACE=1 cargo test --package courier` or `cargo nextest run -p courier`
- Tests automatically spin up isolated test servers with temporary storage and database pools

## Hurry Workflow
1. **For development/testing**: Use `hurry-dev cargo build` after running `make install-dev`
2. **For production use**: Use `hurry cargo build` for all local builds instead of `cargo build`
3. **Drop-in cargo replacement**: `hurry cargo <any-command>` works for all cargo commands
   - Commands with special hurry handling (like `build`) get cache acceleration
   - All other commands pass through to cargo automatically
   - Help and version flags are forwarded to cargo: `hurry cargo --help`, `hurry cargo --version`
4. **Cross compilation support**: `hurry cross <any-command>` passes through to the `cross` tool
5. Use `scripts/ready.sh` to set up a clean testing environment
6. Use the diff scripts to validate cache correctness when making changes
7. Run e2e tests to ensure integration works across different scenarios

## Cargo Command Passthrough
Hurry acts as a drop-in replacement for cargo, supporting any cargo command:

**Accelerated commands** (special hurry handling):
- `hurry cargo build`: Cache-accelerated builds with artifact restore/backup

**Passthrough commands** (forwarded to cargo as-is):
- `hurry cargo check`, `hurry cargo test`, `hurry cargo run`, `hurry cargo clippy`, etc.
- All cargo flags and options are preserved
- Toolchain selection works: `hurry cargo +nightly fmt`
- Cargo plugins work: `hurry cargo machete`, `hurry cargo sqlx prepare`

**Implementation notes:**
- Help/version interception is disabled: `--help` and `--version` are passed to cargo
- Arguments are forwarded exactly as provided
- Only the first argument after `cargo` determines if special handling is needed

## Hurry Cargo Build Flags

`hurry cargo build` accepts all standard `cargo build` flags plus hurry-specific options:

**Hurry-specific flags** (all prefixed with `--hurry-`):
- `--hurry-courier-url <URL>`: Base URL for Courier instance (env: `HURRY_COURIER_URL`, default: staging)
- `--hurry-skip-backup`: Skip backing up the cache
- `--hurry-skip-build`: Skip the cargo build, only perform cache actions
- `--hurry-skip-restore`: Skip restoring the cache
- `--hurry-wait-for-upload`: Wait for all new artifacts to upload before exiting (blocks on daemon uploads)

**Important notes:**
- **Hurry flags MUST come before cargo flags** due to Clap parsing: `hurry cargo build --hurry-wait-for-upload --release` ✅
- Incorrect order will fail: `hurry cargo build --release --hurry-wait-for-upload` ❌
- Regular `cargo build --help` shows cargo's help, not hurry's
- The `--hurry-wait-for-upload` flag is useful for CI/CD to ensure artifacts are fully uploaded

## Courier Workflow
1. Set up environment: `cp .env.example .env` and customize as needed
2. Start PostgreSQL: `docker compose up -d postgres` (or use full `docker compose up` for everything)
3. Apply migrations: `cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url "$COURIER_DATABASE_URL"` (or `docker compose run --build migrate`)
4. Run the server: `courier serve` (reads `COURIER_DATABASE_URL` from `.env` via build.rs)
5. Make API requests: Use curl, xh, httpie, or the test client
6. Iterate on code: Tests use isolated databases via `#[sqlx::test]` macro
7. Schema changes: Edit `schema/schema.sql` → run `sql-schema migration --name {name}` → review migrations → apply with sqlx-cli → run `make sqlx-prepare`

## Release Workflow

Hurry uses S3-based distribution for binary releases. Releases are created manually using a local script.

### Prerequisites
- AWS SSO access configured with appropriate S3 permissions
- `cross` installed for cross-compilation: `cargo install cross`
- `cargo-set-version` installed: `cargo install cargo-set-version`
- `jq` or `jaq` for JSON processing

### Release Process

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

### Release Artifacts

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

### Release Script Options

- `--dry-run`: Build and test without uploading or creating git tags
- `--skip-build`: Use existing artifacts from `target/release-artifacts/`
- `--skip-upload`: Build but don't upload to S3 (useful for testing builds)

### Important Notes

- Version changes in `Cargo.toml` are temporary and not committed
- Prereleases (e.g., `1.0.0-beta.1`) don't update the `/latest/` pointer
- Cache headers are set appropriately: versioned releases are immutable, latest is no-cache
- The script uses `cross` for Linux targets when building from macOS

## Build System Notes

- Uses Rust 2024 edition
- Workspace-based dependency management in root `Cargo.toml`
- **Windows support**: Core functionality works on Windows as of PR #163
  - Cache operations use platform-native directories
  - File metadata operations (mtime, permissions) are cross-platform
  - Daemon architecture refactored for Windows compatibility
  - Some features (like passthrough) fall back to cargo on Windows when needed
  - Release artifacts include Windows binaries (x86_64-pc-windows-gnu only)
- Heavy use of async/await patterns with tokio runtime
- Extensive use of workspace dependencies for consistency
- Courier uses `build.rs` to set `DATABASE_URL` from `COURIER_DATABASE_URL` for sqlx compatibility
- `rust-toolchain.toml` pins toolchains—avoid `rustup override` unless debugging
- Never commit secrets; keep real credentials external
- Clean `target/` and reset caches when benchmarking

## Commit & PR Guidelines

Follow the repository's conventional commit style (`feat: cache warmup (#123)`). Keep commits small, reversible, and lint-clean. PRs need a concise summary, validation or reproduction steps, and screenshots/logs when CLI output shifts. Call out migrations, new env vars, or cache-impacting changes in bold to help reviewers.

Don't commit for me unless I ask you to.
