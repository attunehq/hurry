# Repository Guidelines

This file provides guidance for AI coding assistants (Claude Code, OpenAI Codex, Windsurf, Cursor, Zed, etc) when working with code in this repository.

## Core Principle: Follow Existing Patterns

**IMPORTANT**: When implementing anything, always look for similar examples in the codebase first and follow those patterns.

### Pattern Discovery Process
1. **Before writing code**: Search for similar implementations in the same package/module
2. **When writing tests**: Read 2-3 other tests in the same package/module and follow their structure
3. **When adding features**: Look for analogous features and use similar approaches
4. **When refactoring**: If the user asks to change something in one area and similar patterns exist elsewhere, ask if they'd like you to update those too

### Why This Matters
- **Principle of least surprise**: Code behaves consistently across the codebase
- **Maintainability**: Future developers (human or AI) can understand patterns quickly
- **Quality**: Proven patterns are less likely to have issues

### Examples
- Writing a new API handler? Look at existing handlers in `api/v1/` first
- Adding a new CLI command? Check how other commands in `cmd/` are structured
- Creating a test? Read tests in that module to see assertion styles, setup patterns, and naming conventions
- Implementing serialization? See how other types handle it

## Project Overview

This is a monorepo containing two main projects:

**hurry** is a Rust tool that accelerates Cargo builds by intelligently caching and restoring build artifacts across git branches, worktrees, and development contexts. It provides drop-in replacements for common Cargo commands with significant performance improvements.

**courier** is the API service for Hurry, providing content-addressed storage (CAS) functionality. It's a simple web service built with Axum that handles blob storage with zstd compression, optimized for on-premise deployments where authentication is not a concern. PostgreSQL is available for future distributed caching features.

## Workspace Structure

- `packages/hurry/`: Core hurry implementation, organized as follows:
    - CLI:
        - `src/bin/hurry/main.rs`
        - Command implementations in `src/bin/hurry/cmd/`
    - Caching: `src/cache/`
    - Cargo integration: `src/cargo/`
    - Filesystem operations: `src/fs.rs`
    - Hashing: `src/hash.rs`
    - Daemon: `src/daemon.rs`
    - Cross integration: `src/cross.rs`
- `packages/clients/`: Shared client library providing Courier API types and HTTP client implementations
- `packages/courier/`: API service with API routes (`src/api/`), database (`src/db.rs`), and storage (`src/storage.rs`)
- `packages/e2e/`: End-to-end integration tests package that simulates real-world usage scenarios across git operations, branch switches, and cache restore workflows
- `static/cargo/`: Contains cache markers and metadata for build artifact management
- `scripts/`: Debugging and validation scripts
- `target/`: Build output (do not commit)

### Hurry Components
- Cache system (`packages/hurry/src/cache/`): Manages build artifact caching across different git states
- Cargo integration (`packages/hurry/src/cargo/`): Handles workspace metadata, dependencies, and build profiles
- File operations (`packages/hurry/src/fs.rs`): Optimized filesystem operations with mtime preservation
- Daemon (`packages/hurry/src/daemon.rs`): Background service for async cache uploads and status tracking
- Cargo/Cross integration (`packages/hurry/src/cargo.rs`, `packages/hurry/src/cross.rs`): Command passthrough and build acceleration

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

## Rust Programming Guidelines

This is a Rust codebase. Always follow these conventions when writing or reviewing Rust code.

### Code Style

**String Creation**
- Use `String::from("...")` instead of `"...".to_string()`
- Use `String::new()` instead of `"".to_string()`

**Type Annotations**
- Always use postfix types (turbofish syntax)
- ❌ `let foo: Vec<_> = items.collect()`
- ✅ `let foo = items.collect::<Vec<_>>()`

**Control Flow**
- Prefer `let Some(value) = option else { ... }` over `.is_none()` + `.unwrap()`

**Array Indexing**
- Avoid array indexing. Use iterator methods: `.enumerate()`, `.iter().map()`

**Imports**
- Prefer direct imports over fully qualified paths unless ambiguous
- Never put `import` statements inside functions (unless feature/cfg gated): always put them at file level

**Module Structure**
- Do not use `mod.rs`. Always prefer `my_module.rs` + `my_module/other_file.rs`

**String Formatting**
- Inline rust variables in format strings: `format!("Hello, {name}")`

### Naming Conventions

**Type Names: Avoid Stuttering**
- When a type is namespaced by its module, don't repeat context
- ❌ `storage::CasStorage` or `storage::DiskStorage` — stutters "storage"
- ✅ `storage::Disk` — describes implementation (actual example from courier/src/storage.rs)
- ❌ `db::PostgresDatabase` — stutters "database"
- ✅ `db::Postgres` — describes implementation (actual example from courier/src/db.rs)

**Function and Variable Names**
- Don't prefix test functions with `test_`
- Don't use hungarian notation; prefer shadowing
- ❌ `formats_str` → ✅ `formats`

### Error Handling
- Use `color-eyre` for errors and reporting
- Only panic for invariant violations or in tests
- Prefer returning `Result` for recoverable errors

### Testing Patterns

**Test Organization**
- Colocate tests with code in `#[cfg(test)]` modules (not separate `tests/` directories)
- Write tests integration-style (test public APIs) not unit-style (test internals)

**Assertions**
- Use `pretty_assertions` with prefixed imports to avoid shadowing (see hurry/tests/it/passthrough.rs:7 or hurry/src/cargo/build_args.rs:650):
  ```rust
  use pretty_assertions::assert_eq as pretty_assert_eq;
  ```
- Always construct the ENTIRE expected value upfront and compare in ONE operation
- ✅ Declare expected value first, single assertion
- ❌ Property-by-property assertions

**Parameterized Tests**
- Use `simple_test_case` for tests with multiple variations (see hurry/tests/it/passthrough.rs:55-65 or hurry/src/cargo/build_args.rs:655-662)

**Running Tests**
- Use cargo nextest: `cargo nextest run -p {PACKAGE_NAME}`
- Available packages: `hurry`, `courier`, `clients`, `e2e`

### Development Workflow

**Dependency Management**
- Use `cargo add` instead of manual `Cargo.toml` edits
- Run `cargo autoinherit` after adding packages (workspace uses inheritance)

**Code Quality**
- Format code: `make format` (uses nightly rustfmt)
- Run linter: `make check`
- Auto-fix lints: `make check-fix`
- Pre-commit checks: `make precommit` (runs machete-fix, autoinherit, check-fix, format, sqlx-prepare)

## Context-Specific Guides

For additional patterns and examples beyond the core guidelines above, load these guides:

- **Detailed patterns**: Load `.agents/patterns.md` for architectural patterns including async, HTTP/API, database, file I/O, and type design patterns
- **Development tasks**: Load `.agents/workflow.md` for build commands, testing procedures, release processes, and tool-specific workflows
- **Rust style deep-dive**: Invoke the `rust-programming` skill for detailed examples on naming, control flow, and indexing patterns
- **Testing deep-dive**: Invoke the `rust-testing` skill for detailed assertion patterns and parameterized test examples

**Note**: These guides contain detailed, prescriptive instructions. Only load them when actively working on the relevant task to avoid unnecessary context usage.

## Using Subagents for Large Tasks

For large, isolated tasks that would benefit from focused context, consider using subagents:

- **Large refactoring tasks**: When refactoring entire modules or making sweeping changes across multiple files, use a subagent with the relevant guide loaded (e.g., style.md for code style refactoring)
- **Detailed planning and exploration**: For planning complex features or exploring unfamiliar parts of the codebase, use the Explore subagent to gather information without loading all context into the main conversation
- **Isolated implementation work**: When implementing a well-defined feature that doesn't require extensive back-and-forth, delegate to a subagent with the appropriate guides

### Context Transfer with Subagents

Use `.scratch/` directory to transfer context between main agent and subagents:

1. **Before launching subagent**: Write a context file to `.scratch/{task-name}-context.md` containing:
   - Task description and goals
   - Relevant file paths and locations
   - Constraints and requirements
   - Expected output format

2. **In subagent prompt**: Reference the context file: "Read `.scratch/{task-name}-context.md` for task details and constraints. Also load `.agents/{relevant-guide}.md` for style/pattern guidance."

3. **After subagent completes**: Subagent should write results to `.scratch/{task-name}-results.md` including:
   - Summary of changes made
   - Files modified
   - Any issues or decisions that need review
   - Follow-up tasks or recommendations

4. **Back in main conversation**: Read the results file to integrate subagent's work into the main conversation context

This approach keeps large task context isolated while maintaining continuity across agent boundaries.
