# Design Document: Shared Build Options

## Overview

This design describes the refactoring of shared CLI arguments between `hurry cargo build` and `hurry cross build` commands. The goal is to extract common Hurry-specific options into a shared struct that can be flattened into both command-specific option structs using clap's `#[clap(flatten)]` attribute.

Currently, both `packages/hurry/src/bin/hurry/cmd/cargo/build.rs` and `packages/hurry/src/bin/hurry/cmd/cross/build.rs` define identical fields for Hurry-specific arguments. This duplication creates maintenance burden and risk of divergence.

## Architecture

The refactoring follows a simple extraction pattern:

```
Before:
┌─────────────────────┐    ┌─────────────────────┐
│ cargo/build.rs      │    │ cross/build.rs      │
│ ┌─────────────────┐ │    │ ┌─────────────────┐ │
│ │ Options         │ │    │ │ Options         │ │
│ │ - api_url       │ │    │ │ - api_url       │ │
│ │ - api_token     │ │    │ │ - api_token     │ │
│ │ - skip_backup   │ │    │ │ - skip_backup   │ │
│ │ - skip_build    │ │    │ │ - skip_build    │ │
│ │ - skip_restore  │ │    │ │ - skip_restore  │ │
│ │ - async_upload  │ │    │ │ - async_upload  │ │
│ │ - help          │ │    │ │ - help          │ │
│ │ - argv          │ │    │ │ - argv          │ │
│ └─────────────────┘ │    │ └─────────────────┘ │
└─────────────────────┘    └─────────────────────┘

After:
┌─────────────────────────────────────────────────┐
│ cmd.rs                                          │
│ ┌─────────────────────────────────────────────┐ │
│ │ HurryOptions                                │ │
│ │ - api_url                                   │ │
│ │ - api_token                                 │ │
│ │ - skip_backup                               │ │
│ │ - skip_build                                │ │
│ │ - skip_restore                              │ │
│ │ - async_upload                              │ │
│ └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
           │                         │
           ▼                         ▼
┌─────────────────────┐    ┌─────────────────────┐
│ cargo/build.rs      │    │ cross/build.rs      │
│ ┌─────────────────┐ │    │ ┌─────────────────┐ │
│ │ Options         │ │    │ │ Options         │ │
│ │ #[flatten]      │ │    │ │ #[flatten]      │ │
│ │ hurry: Hurry... │ │    │ │ hurry: Hurry... │ │
│ │ - help          │ │    │ │ - help          │ │
│ │ - argv          │ │    │ │ - argv          │ │
│ └─────────────────┘ │    │ └─────────────────┘ │
└─────────────────────┘    └─────────────────────┘
```

## Components and Interfaces

### HurryOptions Struct

A new struct containing all shared Hurry-specific CLI arguments:

```rust
/// Shared options for Hurry build acceleration.
///
/// These options are common to both `cargo build` and `cross build` commands.
#[derive(Clone, clap::Args, Debug)]
pub struct HurryOptions {
    /// Base URL for the Hurry API.
    #[arg(
        long = "hurry-api-url",
        env = "HURRY_API_URL",
        default_value = "https://app.hurry.build"
    )]
    #[debug("{api_url}")]
    pub api_url: Url,

    /// Authentication token for the Hurry API.
    #[arg(long = "hurry-api-token", env = "HURRY_API_TOKEN")]
    pub api_token: Option<Token>,

    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    pub skip_backup: bool,

    /// Skip the build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    pub skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    pub skip_restore: bool,

    /// Upload artifacts asynchronously in the background instead of waiting.
    ///
    /// By default, hurry waits for uploads to complete before exiting.
    /// Use this flag to upload in the background and exit immediately after the
    /// build.
    #[arg(
        long = "hurry-async-upload",
        env = "HURRY_ASYNC_UPLOAD",
        default_value_t = false
    )]
    pub async_upload: bool,
}
```

### Module Location

The shared struct will be placed directly in `packages/hurry/src/bin/hurry/cmd.rs`. This location:
- Avoids creating a "utility module" that only contains types
- Is directly accessible to both cargo and cross submodules via `super::HurryOptions`
- Follows the principle of keeping related code together

### Updated Options Structs

Both cargo and cross build Options structs will flatten the shared options:

```rust
// In cargo/build.rs
#[derive(Clone, clap::Args, Debug)]
#[command(disable_help_flag = true)]
pub struct Options {
    /// Shared Hurry options.
    #[clap(flatten)]
    pub hurry: super::super::HurryOptions,

    /// Show help for `hurry cargo build`.
    #[arg(long = "hurry-help", default_value_t = false)]
    pub help: bool,

    /// These arguments are passed directly to `cargo build` as provided.
    #[arg(
        num_args = ..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS",
    )]
    argv: Vec<String>,
}
```

## Data Models

No new data models are introduced. The existing `Token` and `Url` types continue to be used.

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Argument Parsing Equivalence

*For any* valid combination of Hurry CLI flags (--hurry-api-url, --hurry-api-token, --hurry-skip-backup, --hurry-skip-build, --hurry-skip-restore, --hurry-async-upload), parsing through the flattened Options struct should produce the same field values as the original non-flattened struct would have.

**Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 3.1, 3.2**

### Property 2: Passthrough Arguments Preservation

*For any* sequence of passthrough arguments (argv), the flattened Options struct should capture them identically to the original struct, preserving order and values.

**Validates: Requirements 3.5, 3.6**

### Property 3: Environment Variable Consistency

*For any* environment variable setting (HURRY_API_URL, HURRY_API_TOKEN, HURRY_ASYNC_UPLOAD), both cargo and cross build commands should resolve the same value when parsed with identical environment state.

**Validates: Requirements 4.3, 4.4**

## Error Handling

No changes to error handling. The existing error handling for missing API tokens and invalid URLs remains unchanged:

- Missing `api_token` when required: Returns error with suggestions for `HURRY_API_TOKEN` env var or `--hurry-api-token` argument
- Invalid `api_url`: Clap's built-in URL parsing handles validation

## Testing Strategy

### Unit Tests

Unit tests will verify:
- `HurryOptions` struct can be parsed with various flag combinations
- Default values are applied correctly when flags are omitted
- Environment variables are respected

### Property-Based Tests

Property-based tests using the `proptest` crate (already used in the codebase) will verify:
- **Property 1**: Generate random valid flag combinations and verify parsing produces expected values
- **Property 2**: Generate random argv sequences and verify they are captured correctly
- **Property 3**: Generate random env var states and verify both commands resolve identically

Each property test should run a minimum of 100 iterations.

Test annotations should follow the format:
```rust
// Feature: shared-build-options, Property 1: Argument Parsing Equivalence
// Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 3.1, 3.2
```

### Integration Tests

Existing integration tests in `packages/hurry/tests/it/` should continue to pass, verifying end-to-end behavior is unchanged.
