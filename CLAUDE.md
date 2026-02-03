# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Hurry is a Rust-based build acceleration tool for Cargo projects. It caches compiled artifacts (units) and restores them to speed up subsequent builds. This fork adds GCP Cloud Storage support for serverless caching.

## Build Commands

```bash
# Build all packages
cargo build

# Build only the hurry CLI
cargo build --package hurry

# Run tests
cargo test

# Run a single test
cargo test --package hurry <test_name>

# Check for compilation errors (faster than build)
cargo check --package hurry

# Build release binary
cargo build --release --package hurry
```

## Architecture

### Package Structure
- `packages/hurry` - Main CLI application and library
- `packages/clients` - HTTP client for Courier API (Hurry's cache server)
- `packages/courier` - Courier cache server implementation

### Key Components in `packages/hurry`

**Cache System:**
- `src/cargo/cache.rs` - `CargoCache` - Courier-based cache (original)
- `src/cargo/gcp_cache.rs` - `GcpCargoCache` - GCP bucket-based cache (this fork)
- `src/gcp_cas.rs` - GCP Cloud Storage CAS (Content-Addressable Storage) implementation

**Storage:**
- Files are stored using content-addressed storage (CAS) with Blake3 hashing
- Content is compressed with zstd before upload
- Unit metadata (fingerprints, file references) is stored as JSON

**Build Flow:**
1. `Workspace::units()` - Compute expected compilation units from build plan
2. Cache restore - Download cached artifacts matching unit hashes
3. `cargo build` - Run the actual build
4. Cache save - Upload newly built artifacts

### GCP Bucket Mode

Set `HURRY_GCP_BUCKET` environment variable to use GCS directly instead of the Hurry API server:

```bash
HURRY_GCP_BUCKET=my-cache-bucket hurry cargo build
```

GCS authentication uses the standard GCP chain:
- `SERVICE_ACCOUNT` environment variable (path to service account JSON)
- gcloud CLI credentials
- GCE metadata service (when running on GCP)

Storage layout in GCS:
- `cas/{xx}/{yy}/{hash}` - Compressed file content (CAS)
- `units/{unit_hash}.json` - Unit metadata (fingerprint, file references)

### Key Types
- `UnitPlan` - Describes a compilation unit to be cached
- `UnitHash` - Content-addressed hash identifying a unit
- `Key` - Blake3 hash used for CAS storage
- `SavedUnit` - Serialized unit metadata for cache storage
- `Fingerprint` - Cargo's fingerprint format (rewritten for portability)

## Nudge

This project uses [Nudge](https://github.com/attunehq/nudge), a collaborative partner that helps you remember coding conventions. Nudge watches your `Write` and `Edit` operations and reminds you about patterns and preferences that matter here—so you can focus on the actual problem instead of tracking stylistic details.

**Nudge is on your side.** When it sends you a message, it's not a reprimand—it's a colleague tapping you on the shoulder. The messages are direct (sometimes blunt) because that's what cuts through when you're focused. Trust the feedback and adjust; if a rule feels wrong, mention it so we can fix the rule.

**Writing new rules:** If the user asks you to add or modify Nudge rules, run `nudge claude docs` to see the rule format, template variables, and guidelines for writing effective messages.

@AGENTS.md
