# Database Management

> [!CAUTION]
> This document is _developer_ documentation. It may be incomplete, and may reflect internal implementation details that are subject to change or have already changed. Rely on this documentation at your own risk.

This workspace contains two separate applications that each need their own database:

1. `courier`: A PostgreSQL-backed API service for content-addressed storage
2. `hurry`: A SQLite-backed build cache manager

Normally, sqlx expects a single `DATABASE_URL` environment variable, which doesn't work when you have multiple databases in one workspace. Before sqlx 0.9, this was effectively unsolvable without awkward workarounds like maintaining separate `.env` files or constantly switching environment variables.

For this reason, we're using the `sqlx` v0.9 alpha.

> [!TIP]
> Even though this is an alpha, `sqlx` itself is not alpha software. The "alpha" here really refers to the multi-database management. Given this we're not inflicting alpha-quality software on our users, only ourselves 🥲

### How It Works

Each package has a `sqlx.toml` file in its root directory:

**`packages/courier/sqlx.toml`**:
```toml
[common]
database-url-var = "COURIER_DATABASE_URL"

[migrate]
migrations-dir = "schema/migrations"
```

**`packages/hurry/sqlx.toml`**:
```toml
[common]
database-url-var = "HURRY_DATABASE_URL"
```

When sqlx-cli or the sqlx macros run, they:
1. Look for `sqlx.toml` in the current directory
2. Read the `database-url-var` setting
3. Use that environment variable instead of `DATABASE_URL`

> [!CAUTION]
> The `#[sqlx::test]` macro specifically doesn't yet have support for `sqlx.toml`, so we work around this using `build.rs` for now. Once this issue is fixed upstream we can remove the workaround.

## Getting Set Up

### 1. Initialize Git Submodules

First, fetch the vendored sqlx submodule:

```bash
git submodule update --init --recursive
```

This clones the sqlx repository into `vendor/sqlx`.

### 2. Install sqlx-cli

Install sqlx-cli from the vendored submodule:

```bash
cargo install --path vendor/sqlx/sqlx-cli --features postgres,sqlite,sqlx-toml
```

The `postgres` and `sqlite` features let you work with both databases in this workspace. The `sqlx-toml` feature enables multi-database support.

### 3. Configure Environment Variables

Copy the example environment file and customize it:

```bash
cp example.env .env
```

The `.env` file contains:
```bash
COURIER_DATABASE_URL=postgres://courier:courier@localhost:5432/courier
HURRY_DATABASE_URL=sqlite:.scratch/hurry/hurry.db
CAS_ROOT=.scratch/courier/cas
```

Adjust the URLs as needed for your local setup.

### 4. Start the Databases

For courier's PostgreSQL:
```bash
docker compose up -d postgres
```

For hurry's SQLite, no server is needed: the database file is created automatically when you run migrations or tests.

### 5. Run Migrations

sqlx-cli discovers the `sqlx.toml` configuration in the current directory, so you must `cd` into each package directory before running commands:

```bash
# Courier migrations
cd packages/courier
cargo sqlx migrate run

# Hurry migrations
cd packages/hurry
cargo sqlx migrate run
```

## Working with sqlx-cli

All sqlx-cli commands work the same way: run them from within the package directory.

### Common Commands

**Create a new migration**:
```bash
cd packages/courier
cargo sqlx migrate add my_migration_name
```

**Run migrations**:
```bash
cd packages/courier
cargo sqlx migrate run
```

**Revert last migration**:
```bash
cd packages/courier
cargo sqlx migrate revert
```

**Prepare for offline mode** (generates `.sqlx/` files for CI):
```bash
cd packages/courier
cargo sqlx prepare --workspace
```

**Create database** (if it doesn't exist):
```bash
cd packages/courier
cargo sqlx database create
```

**Drop database** (destructive!):
```bash
cd packages/courier
cargo sqlx database drop
```

### Why the `cd` is Required

sqlx-cli doesn't have a flag to specify which `sqlx.toml` to use. It simply looks in the current directory and walks up the directory tree until it finds one. By running commands from within each package directory, you ensure it picks up the right configuration.

The sqlx team acknowledges this isn't ideal for multi-database workspaces (ref: [sqlx#3761](https://github.com/launchbadge/sqlx/issues/3761)), but it's the current state of the alpha.

## Running Tests

Tests automatically use the correct database based on each package's `sqlx.toml`, so you don't need to `cd` or specify anything special:

```bash
# From workspace root
cargo nextest run -p courier  # Uses COURIER_DATABASE_URL
cargo nextest run -p hurry     # Uses HURRY_DATABASE_URL
```

The `#[sqlx::test]` macro in courier automatically:
1. Creates an isolated test database
2. Runs migrations
3. Loads fixtures
4. Runs your test
5. Tears down the test database

Each test gets its own database instance, so they can run in parallel without interfering.

> [!CAUTION]
> The `#[sqlx::test]` macro specifically doesn't yet have support for `sqlx.toml`, so we work around this using `build.rs` for now. Once this issue is fixed upstream we can remove the workaround.

## Courier-Specific Workflow

Courier uses a PostgreSQL database with a hand-maintained schema file and generated migrations.

### Schema Changes

1. Edit `packages/courier/schema/schema.sql` with your desired changes
2. Generate a migration: `sql-schema migration --name my_change_name`
3. Review the generated migration files in `packages/courier/schema/migrations/`
4. Apply the migration: `cd packages/courier && cargo sqlx migrate run`
5. Update `.sqlx/` files if needed: `cd packages/courier && cargo sqlx prepare`

### Running the Server

The server reads `COURIER_DATABASE_URL` from `.env`:

```bash
courier serve
```

Or via Docker (which handles migrations automatically):

```bash
docker compose up
```

## Hurry-Specific Workflow

Hurry uses SQLite for local caching metadata. The database usage is still in early development.

When hurry's database features are implemented, the workflow will be similar to courier's but with SQLite-specific considerations:

- SQLite creates the database file automatically if it doesn't exist
- No server process is needed
- Migrations can be run the same way: `cd packages/hurry && cargo sqlx migrate run`

## Why Vendor sqlx?

We vendor sqlx 0.9 as a git submodule pinned to a specific commit for **stability**.

Using an unreleased alpha version directly (via git dependency or similar) means:
- Every developer could have a different version depending on when they last updated
- Alpha changes can introduce breaking changes without warning
- Builds become non-reproducible across machines and time
- CI and local development can drift apart

By vendoring as a git submodule:
1. All developers use the exact same sqlx commit
2. We control when to update
3. Builds are reproducible
4. Consistent sqlx-cli across machines

## Troubleshooting

### "DATABASE_URL must be set"

This usually means you're running a sqlx-cli command from the wrong directory. Make sure you've `cd`'d into the package directory (e.g., `packages/courier` or `packages/hurry`).

### "database doesn't exist"

Run `cargo sqlx database create` from within the package directory:

```bash
cd packages/courier
cargo sqlx database create
```

### "unknown feature: sqlx-toml"

You're using an older version of sqlx-cli. Make sure you installed from the vendored submodule:

```bash
cargo install --path vendor/sqlx/sqlx-cli --features postgres,sqlite,sqlx-toml --force
```

### Test failing with "connection refused"

For courier tests, make sure PostgreSQL is running:

```bash
docker compose up -d postgres
```

For hurry tests, this shouldn't happen since SQLite doesn't need a server. If you see this, check that `HURRY_DATABASE_URL` is set correctly in `.env`.
