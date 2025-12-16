# Development Container

> [!CAUTION]
> This document is _developer_ documentation. It may be incomplete, and may reflect internal implementation details that are subject to change or have already changed. Rely on this documentation at your own risk.

## Overview

The hurry repository includes a Docker-based development container that provides a consistent, isolated environment for development. This container comes preconfigured with all necessary tools, dependencies, and environment variables, allowing you to develop without polluting your host system or dealing with dependency conflicts.

## Setup

### Prerequisites

- Docker and Docker Compose installed
- Host machine running the hurry repository

### Initial Setup

1. Ensure you have a `.env` file in the repository root (copy from `.env.example` if needed)
2. Build and start the dev container:
   ```bash
   docker compose build dev
   docker compose up -d dev
   ```
3. Enter the container:
   ```bash
   docker compose exec dev zsh
   ```

> [!NOTE]
> We use `docker compose up -d` + `docker compose exec` rather than `docker compose run --rm` because the service approach keeps the container running in the background, allows multiple shell attachments, and properly respects `depends_on` relationships with postgres. The `run` command is better suited for one-off tasks.

### Persisted State

The following directories and files are persisted across container rebuilds via Docker volumes:

- `~/.claude/` and `~/.claude.json`: Claude Code configuration and authentication
- `~/.config/gh/`: GitHub CLI configuration and authentication
- `~/.ssh/`: SSH keys and known_hosts for git operations
- Cargo registry and git caches: Speeds up dependency downloads
- Shell history: Command history is preserved between sessions
- `target/`: Build artifacts (isolated from host)

## Available Tools

The dev container includes:

**Build Tools:**
- Rust toolchain (via rustup)
- Node.js (via Volta)
- cargo-binstall: Fast binary installation for Rust tools
- make: For running Makefile targets

**Rust Development:**
- cargo-nextest: Faster test runner
- cargo-machete: Find unused dependencies
- cargo-autoinherit: Manage workspace dependencies
- sqlx-cli: Database migrations
- clippy, rustfmt: Linting and formatting (via rustup)

**CLI Tools:**
- eza: Modern replacement for ls
- ripgrep: Fast text search
- fd-find: Fast file search
- bat: Better cat with syntax highlighting
- starship: Cross-shell prompt
- gh: GitHub CLI
- psql: PostgreSQL client
- Claude Code: AI coding assistant

## Environment Variables

The container automatically sets the following environment variables:

```bash
# Database
COURIER_DATABASE_URL=postgres://courier:courier@postgres:5432/courier
PGPASSWORD=courier

# Hurry API URLs
HURRY_API_URL=https://courier.staging.corp.attunehq.com  # Default
HURRY_API_URL_STAGING=https://courier.staging.corp.attunehq.com
HURRY_API_URL_DOCKER=http://courier:3000  # Container-to-container
HURRY_API_URL_LOCAL=http://host.docker.internal:3000  # Container-to-host

# Development
RUST_BACKTRACE=1
```

Additional environment variables from your `.env` file are also loaded.

## Aliases

The container provides convenient aliases for common development tasks:

**File Operations:**
- `ls`: eza
- `ll`: eza -l --git -b (detailed list with git status)
- `lt`: eza -Tlm --time-style long-iso (tree view)
- `lta`: eza -Tlma --time-style long-iso (tree view with hidden files)

**Navigation:**
- `..`: cd ..

**Rust Development:**
- `cb`: cargo build
- `ct`: cargo test
- `ntr`: cargo nextest run
- `ntrl`: RUST_LOG=info cargo nextest run --no-capture
- `ntrd`: RUST_LOG=debug cargo nextest run --no-capture
- `ntrlt`: RUST_LOG=trace cargo nextest run --no-capture
- `hurry_install`: cargo install --path ./packages/hurry --locked

**Database:**
- `psql`: psql -h postgres -U courier -d courier
- `courier_migrate`: cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url postgres://courier:courier@postgres:5432/courier

**Git:**
- `gc`: git clone

**AI Assistant:**
- `claude`: claude --dangerously-skip-permissions (runs in "yolo mode")

## Networking

The dev container can communicate with:

1. **Other services in the compose stack**: Use service names directly (e.g., `http://courier:3000`, `postgres`)
2. **Host machine services**: Use `host.docker.internal` (e.g., `http://host.docker.internal:8080`)
3. **External services**: Standard internet connectivity works as expected

The container depends on the `postgres` service and will not start until postgres is healthy.

## Workflow

### Standard Development

1. Enter the container:
   ```bash
   docker compose exec dev zsh
   ```

2. Make changes to code on your host machine (they're synced via volume mount)

3. Build and test inside the container:
   ```bash
   cb  # cargo build
   ntr -p hurry  # run tests
   ```

4. Access the database:
   ```bash
   psql  # connects to postgres container
   courier_migrate  # apply migrations
   ```

### Using Claude Code

The container includes Claude Code preconfigured with your authentication:

```bash
claude  # starts interactive session with full permissions
```

Your Claude configuration from `~/.claude/` is automatically available.

### Installing Additional Tools

Since the dev user has passwordless sudo, you can install additional packages:

```bash
sudo apt-get update && sudo apt-get install -y some-package
```

For Rust binaries, use cargo-binstall for faster installation:

```bash
cargo binstall -y some-rust-tool
```

To persist these changes across container rebuilds, add them to `docker/dev/Dockerfile`.

## Troubleshooting

### Permission Errors

If you encounter permission errors with the `target/` directory:

```bash
# From host
docker compose down -v  # Remove volumes
docker compose up -d dev  # Recreate with correct permissions
```

This can happen if the volumes were created while the container was running as a different user.

### Build Cache Issues

The container uses a separate `target/` directory from your host builds. This means:
- Host builds go to `target/` on your filesystem
- Container builds go to a Docker volume (never visible on host)
- No conflicts, no shared state

If you need to clear the container's build cache:

```bash
# From inside container
cargo clean

# Or from host (nuclear option)
docker compose down -v  # Removes all volumes
```

### Authentication

Some tools may store logins slightly differently depending on the system, even when configs are shared. In these cases you may need to log in again.

## Architecture Notes

- The container runs as a non-root user (`dev`) with passwordless sudo access
- All Rust tooling is installed directly in the dev user's home directory
- The workspace is mounted at `/workspace` with full read/write access
- An entrypoint script fixes volume permissions on startup to prevent issues
- The container uses zsh with starship prompt for a comfortable development experience
