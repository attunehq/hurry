---
name: rust-programming
description: Rust programming style guide and conventions. Use this skill when writing, reviewing, or modifying Rust code. Covers string creation, type annotations, control flow, naming conventions, imports, error handling, and Rust-specific best practices.
---

# Rust Programming Style Guide

Quick reference for project-specific Rust conventions. Load reference files for detailed examples.

## Code Style Quick Reference

### String Creation
- Use `String::from("...")` instead of `"...".to_string()`
- Use `String::new()` instead of `"".to_string()`

### Type Annotations
- Always use postfix types (turbofish syntax)
- ❌ `let foo: Vec<_> = items.collect()`
- ✅ `let foo = items.collect::<Vec<_>>()`

### Control Flow
Prefer `let Some(value) = option else { ... }` over `.is_none()` + `.unwrap()`

### Array Indexing
Avoid array indexing. Use iterator methods: `.enumerate()`, `.iter().map()`

**See** `references/indexing-patterns.md` for detailed examples

## Naming Conventions Quick Reference

### Type Names: Avoid Stuttering
When a type is namespaced by its module, don't repeat context:
- ❌ `storage::CasStorage` or `storage::DiskStorage` — stutters "storage"
- ✅ `storage::Disk` — describes implementation (see courier/src/storage.rs:54)
- ❌ `db::PostgresDatabase` — stutters "database"
- ✅ `db::Postgres` — describes implementation (see courier/src/db.rs:29)

### Enum Variants
Use single canonical variant with alias functions instead of separate variants for the same concept

**See** `references/naming.md` for complete guidelines

### Function and Variable Names
- Don't prefix test functions with `test_`
- Don't use hungarian notation; prefer shadowing
- ❌: `formats_str` → ✅: `formats`

## Import and Module Style

### Imports
Prefer direct imports over fully qualified paths unless ambiguous:
```rust
// ✅ Prefer
use client::courier::v1::{Key, cache::ArtifactFile};
let key = Key::from_hex(&hex_string)?;

// ❌ Avoid
let key = client::courier::v1::Key::from_hex(&hex_string)?;
```

### Module Structure
- Do not use `mod.rs`. Always prefer `my_module.rs` + `my_module/other_file.rs`

### String Formatting
Inline rust variables in format strings if possible:
- ✅ `format!("Hello, {name}")`
- Use expressions when needed: `format!("Hello, {}", user.name())`

## Development Workflow

### Dependency Management
- Use `cargo add` instead of manual `Cargo.toml` edits
- Run `cargo autoinherit` after adding packages (if workspace uses inheritance)

### Code Quality
- Format code: `make format` (uses nightly rustfmt)
- Run linter: `make check`
- Auto-fix lints: `make check-fix`
- Pre-commit checks: `make precommit` (runs machete-fix, autoinherit, check-fix, format, sqlx-prepare)
- Prefer `Itertools::sorted` over `Vec::sort` for iterator chains

### I/O Operations
Prefer streaming over buffered by default. Use traits: `AsyncRead`, `AsyncWrite`, `Read`, `Write`

### Error Handling
- Use `color-eyre` for errors and reporting
- Only panic for invariant violations or in tests
- Prefer returning `Result` for recoverable errors

### Documentation
Don't bold bullet points in markdown:
- ❌ `- **Hook**: message`
- ✅ `- Hook: message`

Avoid "space dash space" pattern in prose, use colon instead:
- ❌ "All commands work the same way - do x then y"
- ✅ "All commands work the same way: do x then y"

## Reference Files

Load these for detailed examples and patterns:

- `references/indexing-patterns.md` - Array indexing alternatives with iterator methods
- `references/naming.md` - Complete naming conventions with examples
- `references/control-flow.md` - Control flow patterns and examples

## When to Use This Skill

Invoke when:
- Writing new Rust code
- Reviewing code for style violations
- Refactoring existing Rust code
- Onboarding to project conventions
