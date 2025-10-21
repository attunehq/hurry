# RFC 0002: Shared Client Library

## Overview

This RFC proposes extracting the Courier HTTP client from the `hurry` package into a new shared `client` library crate. The primary goal is to eliminate type duplication between the client (in `hurry`) and server (in `courier`) while establishing a pattern for future API clients.

The shared library will contain both the type definitions used in Courier's API and the HTTP client implementation. Types will always be available, while the HTTP client code will be gated behind a `client` feature flag.

## Motivation

### Current State

Today, API types are duplicated between `hurry` and `courier`:

- `hurry::client::ArtifactFile` vs `courier::api::v1::cache::cargo::ArtifactFile`
- `hurry::client::CargoSaveRequest` vs types in courier's handlers
- `hurry::client::CasBulkWriteResponse` vs `courier::api::v1::cas::bulk::write::BulkWriteResponseBody`
- `hurry::hash::Blake3` (hex string) vs `courier::storage::Key` (bytes)
- `courier` tests use bespoke `serde_json` types.

This duplication creates maintenance burden and risks API drift between client and server. When the API changes, we must update types in multiple places and ensure they stay in sync.

### Goals

1. Single source of truth for API types
2. Type-safe client-server communication enforced by the compiler
3. Minimal compilation overhead for `courier` (which doesn't need HTTP client code)
4. Foundation for future API clients (GitHub, NPM, etc.)
5. Support for API versioning (v1, v2, etc.)

### Non-Goals

1. Backward compatibility: This is a monorepo, so breaking changes are acceptable
2. Per-API feature flags: Only one `client` feature for now (can be added later)
3. Multiple transport layers: HTTP only for now

## Design

### Crate Structure

```
packages/client/
├── Cargo.toml
└── src/
    ├── lib.rs
    └── courier/
        ├── v1.rs # Core types (Key, SerializeString)
        └── v1/
            ├── cas.rs        # CAS-specific types
            ├── cache.rs      # Cache-specific types
            └── client.rs     # HTTP client (behind "client" feature)
```

> [!IMPORTANT]
> Per project conventions, modules use `{module_name}.rs` files instead of `mod.rs` and core types/functions used by a module go in the module file.

Future expansion would look like:

```
packages/client/src/
├── courier/...
├── github/          # Future: GitHub API client, maybe feature gated
│   ├── v1.rs
│   └── v1/...
└── npm/             # Future: NPM registry client, maybe feature gated
    ├── v1.rs
    └── v1/...
```

### Module Organization

**`packages/client/src/lib.rs`:**
```rust
pub mod courier;

/// The latest Courier client version.
pub type Courier = courier::v1::Client;

/// Courier v1 client.
pub type CourierV1 = courier::v1::Client;

// Future:
// pub mod github;
// pub type GitHub = github::v1::Client;
```

**`packages/client/src/courier.rs`:**
```rust
pub mod v1;

// Future:
// pub mod v2;
```

**`packages/client/src/courier/v1.rs`:**
```rust
pub mod cas;
pub mod cache;

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
pub use client::Client;

/// The key to a content-addressed storage blob.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display, From)]
#[display("{}", self.to_hex())]
pub struct Key(Vec<u8>);

// Implementation for `Key`
// Other shared types and their implementations
// Other shared functions, etc
```

### Type Definitions

**`packages/client/src/courier/v1.rs`:**

- `Key`: Blake3 hash stored as `Vec<u8>` (adopts semantics from `courier::storage::Key`)
  - Methods: `from_hex()`, `to_hex()`, `as_bytes()`, `from_blake3_hash()`, `from_buffer()`, `from_fields()`
  - Serializes as hex string for JSON API (via serde)
  - Uses byte representation internally for efficiency
  - Validates hash length is 32 bytes in `from_hex()`

**`packages/client/src/courier/v1/cas.rs`:**

CAS-specific API types:

- `CasBulkWriteResponse { written: HashSet<Key>, skipped: HashSet<Key>, errors: HashSet<BulkWriteKeyError> }`
- `BulkWriteKeyError { key: Key, error: String }`
- `CasBulkReadRequest { keys: Vec<Key> }`

**`packages/client/src/courier/v1/cache.rs`:**

Cargo cache API types:

- `ArtifactFile { object_key: Key, path: String, mtime_nanos: u128, executable: bool }`
  - `object_key`: Uses `Key` type directly (was `String`)
  - `path`: Stores JSON-encoded `QualifiedPath` as `String` (manual serialization)
- `CargoSaveRequest { package_name, package_version, target, library_crate_compilation_unit_hash, build_script_compilation_unit_hash, build_script_execution_unit_hash, content_hash, artifacts: Vec<ArtifactFile> }`
- `CargoRestoreRequest { package_name, package_version, target, library_crate_compilation_unit_hash, build_script_compilation_unit_hash, build_script_execution_unit_hash }`
- `CargoRestoreResponse { artifacts: Vec<ArtifactFile> }`

**Implementation note:** Rather than using a generic `SerializeString<T>` wrapper, the implementation uses manual JSON serialization for paths. This simplified the type system and avoided complex serde bounds issues while maintaining the same functionality.

**`packages/client/src/courier/v1/client.rs`** (behind `client` feature):

- `Client`: HTTP client struct using `reqwest`
- Methods for all Courier API endpoints:
  - CAS: `cas_exists()`, `cas_read()`, `cas_write()`, `cas_write_bytes()`, `cas_read_bytes()`, `cas_write_bulk()`, `cas_read_bulk()`
  - Cache: `cargo_cache_save()`, `cargo_cache_restore()`
  - Health: `ping()`

### Usage Examples

**In `hurry` (with `client` feature):**

```rust
use client::courier::v1::{Client, Key};
// Or use the alias:
use client::Courier;

let courier = Courier::new(base_url)?;
courier.cas_write(&key, content).await?;
```

**In `courier` (types only, no `client` feature):**

```rust
use client::courier::v1::{Key, ArtifactFile, CargoSaveRequest};

pub async fn handle(Json(request): Json<CargoSaveRequest>) -> Response {
    // Use shared types directly in API handlers
}

#[cfg(test)]
mod test {
    // Use shared types directly in tests
}
```

**Future GitHub client example:**

```rust
use client::GitHub;

let github = GitHub::new(token)?;
let repo = github.get_repository("owner/repo").await?;
```

### Feature Flags

The crate has a single `client` feature that gates HTTP client dependencies:

```toml
[features]
default = []
client = [
    "dep:reqwest",
    "dep:tokio",
    "dep:tokio-util",
    "dep:futures",
    "dep:color-eyre",
    "dep:url",
    "dep:async-tar",
    "dep:piper",
    "dep:tap",
    "dep:tracing"
]
```

This ensures:
- `courier` can use types without pulling in HTTP client dependencies
- `hurry` gets the full HTTP client by enabling the feature
- Future granular feature flags can be added as needed (e.g., `github` to enable a github client)

The assumption is that this library will _always_ provide `courier` API types, and `client` will _always_ enable the `courier` client: "courier" is the _main_ client type for this library. But maybe in the future we'll want to pick and choose other client types and implementations to actually compile- for example we might only compile the `github` client and its types if the `github` feature is enabled.

### Key Type Semantics and Migration

The shared `Key` type adopts the semantics from `courier::storage::Key`:

- Internal representation: `Vec<u8>` (32 bytes for Blake3)
- Serialization: Hex string for JSON API (automatically via serde)
- Conversion: `from_hex()` and `to_hex()` methods, plus `from_blake3_hash()`, `from_buffer()`, `from_fields()`

This differs from the original `hurry::hash::Blake3` which stored the hex string directly. The byte representation was chosen because:

1. More efficient for storage and comparison
2. Matches courier's existing implementation
3. Smaller memory footprint
4. Canonical representation is bytes, not hex

**Migration in hurry:**
- Removed `hurry::hash::Blake3` type entirely
- Updated `hurry::hash::hash_file()` to return `Key` directly
- Updated all internal usages to work with `Key`
- Created type conversions at API boundaries (e.g., in `CourierCas` methods)

## Migration Plan

### Step 1: Create the new crate

- Add `packages/client/Cargo.toml` with dependencies and feature flags
- Create module structure: `lib.rs` → `courier.rs` → `v1.rs` → modules
- Add type aliases in `lib.rs` for ergonomic imports

### Step 2: Extract shared types

- Move `Key` from `courier::storage` to `client::courier::v1`
  - Keep `Vec<u8>` internal representation
  - Ensure hex serialization for JSON API
  - Add conversion methods: `from_hex()`, `to_hex()`, `from_blake3_hash()`, `from_buffer()`, `from_fields()`
- Create `client::courier::v1::cas` module with CAS types
- Create `client::courier::v1::cache` module with cache types
- Deduplicate `ArtifactFile` and request/response types
  - Update `ArtifactFile::object_key` from `String` to `Key`
  - Use manual JSON serialization for path field (instead of generic `SerializeString<T>`)

### Step 3: Extract client code

- Move HTTP client implementation from `hurry::client` to `client::courier::v1::client`
- Rename struct to `Client`
- Update imports to use local types from `client::courier::v1`
- Gate behind `client` feature flag

### Step 4: Update `courier` to use shared types

- Add `client` dependency with default features only (no `client` feature)
- Replace `courier::storage::Key` with `client::courier::v1::Key`
- Replace API handler types with `client::courier::v1::{cas::*, cache::*}`
- Update database layer to use shared `Key` type
- Remove duplicate type definitions from `courier::api::v1`

### Step 5: Update `hurry` to use shared client

- Add `client` dependency with `client` feature enabled
- Replace `hurry::client::Courier` with `client::Courier` (or `client::courier::v1::Client`)
- Update `hurry::cas` to use new client module
- Keep `hurry::hash::Blake3` for hurry-specific functionality
  - Add `impl From<hurry::hash::Blake3> for client::courier::v1::Key`
  - Convert at API call boundaries
- Remove old `hurry::client` module

### Step 6: Testing

- Run all package tests: `cargo nextest run --workspace`
- Verify courier tests pass with shared types
- Verify hurry tests pass with shared client
- Run e2e integration tests
- Check for compilation warnings

## Dependencies

**`packages/client/Cargo.toml`:**

```toml
[package]
name = "client"
version = "0.0.0"
edition = "2024"

[dependencies]
# Always available (for types)
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
bon = { workspace = true }
derive_more = { workspace = true, features = ["full"] }
hex = { workspace = true }
blake3 = { workspace = true }
color-eyre = { workspace = true }

# Only with "client" feature
reqwest = { workspace = true, features = ["json", "stream", "rustls-tls", "gzip", "brotli"], optional = true }
tokio = { workspace = true, features = ["full"], optional = true }
tokio-util = { workspace = true, features = ["full"], optional = true }
futures = { workspace = true, optional = true }
url = { workspace = true, optional = true }
async-tar = { workspace = true, optional = true }
piper = { workspace = true, optional = true }
tap = { workspace = true, optional = true }
tracing = { workspace = true, optional = true }
flume = { workspace = true, optional = true }

[features]
default = []
client = [
    "dep:reqwest",
    "dep:tokio",
    "dep:tokio-util",
    "dep:futures",
    "dep:url",
    "dep:async-tar",
    "dep:piper",
    "dep:tap",
    "dep:tracing",
    "dep:flume"
]
```

**Notes on dependencies:**
- `blake3` and `color-eyre`: Always included (needed for `Key` type methods)
- `flume`: Added to support bulk read operations with channel-based streaming
- All HTTP and async dependencies are behind the `client` feature flag

**Update `packages/hurry/Cargo.toml`:**

```toml
[dependencies]
client = { path = "../client", features = ["client"] }
# Remove: reqwest, some tokio-util features used only for client
```

**Update `packages/courier/Cargo.toml`:**

```toml
[dependencies]
client = { path = "../client" }
# Types available, no HTTP client code compiled
```

## Future Extensibility

The design supports future expansion with minimal changes:

### Adding API versions

```rust
// In courier.rs
pub mod v1;
pub mod v2;

// Make v2 the default
pub use v2::*;
```

### Adding per-API feature flags

```toml
[features]
github = []
```

### Adding new API clients

```
src/
├── courier/...
├── github/
│   ├── v1.rs
│   └── v1/
│       ├── client.rs
│       └── ...
└── npm/
    └── v1.rs
    └── v1/
        ├── client.rs
        └── ...
```

With corresponding type aliases in `lib.rs`:

```rust
pub type GitHub = github::v1::Client;
pub type Npm = npm::v1::Client;
```

## Benefits

1. API types defined once, used by both client and server
2. Compiler enforces client-server compatibility
3. Courier compiles only types, not HTTP client
4. Library structure mirrors API versioning
5. Easy to add new API versions and clients
6. Type aliases provide convenient access patterns
7. Eliminates several hundred lines of mostly duplicate type definitions

## Alternatives Considered

### Alternative 1: Keep types in `courier`, reference from `hurry`

This would make `hurry` depend on `courier`, creating a backwards dependency. We rejected this because:
- Courier is the server, it shouldn't be a dependency of clients
- Future external clients couldn't depend on the server package
- Doesn't support future API clients like GitHub

### Alternative 2: Separate crate per API client

Create `courier-client`, `github-client`, etc. as separate crates. We rejected this because:
- More crates to manage and version
- Doesn't share common HTTP client infrastructure
- Harder to maintain consistency across clients
