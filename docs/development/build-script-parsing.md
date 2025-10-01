# Build Script Caching Strategy

## The Problem

We have a chicken-and-egg problem with caching build artifacts:

1. **To back up artifacts**: We need cache keys derived from build script outputs
2. **To restore artifacts**: We need to reconstruct expected cache keys WITHOUT running build scripts

This is critical because some build scripts compile C libraries, taking 10s-100s of seconds. If we can't cache these effectively, it severely weakens the value proposition of `hurry`.

> [!TIP]
> One example is [`aws-lc-sys`](https://github.com/aws/aws-lc-rs/blob/main/aws-lc-sys/builder/main.rs), which on my system (MacBook Pro with an M4 Pro SoC) takes 19 seconds to build; on other systems we've seen it take _70 seconds!_

### Why Cache Keys Depend on Build Script Outputs

Consider this simplified example from `openssl-sys`:

> [!NOTE]
> This is a toy example. The real build script is much more complicated:
> https://github.com/sfackler/rust-openssl/blob/master/openssl-sys/build/main.rs

```rust
// Inside build.rs
fn main() {
    let openssl_dir = find_openssl(); // System-dependent!
    let openssl = openssl_dir.join("openssl");
    println!("cargo:rerun-if-changed={openssl}");
    println!("cargo:include={openssl_dir}");
    println!("cargo:rustc-link-search=native={openssl_dir}");
    println!("cargo:rustc-link-lib=native={openssl}");
}
```

The build script output varies by machine: `find_openssl()` might return `/usr/lib/openssl` on one system and `/opt/homebrew/lib/openssl` on another. Same crate version, different artifacts, different cache keys needed, and we can't really predict this without running the script.

### Why Static Analysis is Hard

Fully modeling build scripts is effectively equivalent to modeling a full execution environment, as build scripts can execute arbitrary code. We can't simply hash the build script source because build scripts often probe system state (finding libraries, reading env vars, checking OS versions, etc.) and produce different outputs on different machines despite having identical source code.

Complex real-world examples like `ring` demonstrate why static analysis is intractable:
- `aws-lc-sys` build script: https://github.com/aws/aws-lc-rs/blob/main/aws-lc-sys/builder/main.rs
- `openssl-sys` build script: https://github.com/sfackler/rust-openssl/blob/master/openssl-sys/build/main.rs
- `ring` build script: https://github.com/briansmith/ring/blob/main/build.rs

## Our Solution: Shotgun Restore

Rather than trying to predict which build script outputs we need, we use a "shotgun restore" approach:

1. **During backup**: Store ALL variants of build script outputs we encounter across machines/builds/compiler options/etc.
  - The cache is shared across machines/developers, so variants from different environments accumulate over time.
  - Mitigation strategies for this accumulation are discussed further in this document.
2. **During restore**: Restore ALL stored variants for a given dependency to the local machine prior to building.
3. **Let Cargo decide**: Cargo validates restored artifacts and picks the correct variant.

This trades bandwidth/storage (cheap) for implementation complexity (expensive).

### How It Works

When backing up after a build:
```rust
// Pseudocode
fn backup_build_artifacts(dep: &Dependency) -> Result<()> {
    // Enumerate the artifacts produced for both compiling and running the build
    // script for the dependency.
    let artifacts = find_build_artifacts(dep)?;

    for artifact in artifacts {
        // For each artifact, extract the hash cargo gives it; e.g. the
        // `0f04e1688d488acd` in `ring-0f04e1688d488acd`. These are stored in
        // the path for the artifact and sometimes in other structured output
        // files.
        let hash = read_cargo_hash(&artifact)?;

        // We then store the artifact in the cache in a special namespace that
        // allows us to store multiple build artifact variants per dependency
        // cache key. The idea is that we use multiple fields to compute the
        // cache key for the dependency, and then we can append arbitrary number
        // of artifacts for build scripts to that cache key.
        //
        // The hash is only there to prevent storing duplicate build script
        // artifacts for the same dependency.
        cache.store_build_variant(dep, hash, artifact)?;
    }

    Ok(())
}
```

When restoring before a build:
```rust
// Pseudocode
fn restore_build_artifacts(dep: &Dependency) -> Result<()> {
    // We just retrieve all variants we've ever seen for this dependency cache
    // key and restore them all. Assuming one matches what cargo thinks should
    // be there, it's used for the build; otherwise cargo builds a new variant.
    let variants = cache.list_build_variants(dep)?;
    for variant in variants {
        cache.restore_variant(dep, variant)?;
    }

    Ok(())
}
```

### Key Insight: Cargo Validates Everything

Cargo already has sophisticated logic to determine if build artifacts are valid. When we restore multiple variants:
- Cargo checks timestamps, hashes, and metadata
- Cargo ignores artifacts that don't match the current build configuration
- Cargo rebuilds only what's actually invalid

This means **false positives don't cause incorrect builds**, they only waste bandwidth. False negatives (missing the right variant) just mean cargo rebuilds, which is the fallback anyway.

> [!TIP]
> Cargo's validation is why this approach is safe. We're not trying to outsmart cargo; we're just pre-populating its cache and letting it do what it already does.

### Demonstration

We have e2e "proof-of-concept" tests:
- `packages/e2e/tests/it/experimental/local.rs`: Tests on local machine
- `packages/e2e/tests/it/experimental/docker/debian.rs`: Tests in Docker containers

The tests:
1. Build a project with different feature combinations (e.g., `["bundled-sqlite"]`, `["static-openssl"]`, `["bundled-sqlite", "static-openssl"]`)
2. Back up build outputs from each configuration into a unified target directory
3. Clean the workspace and restore the unified target directory
4. Build again with each feature combination and verify **all** third-party artifacts are fresh

This indicates cargo correctly identifies and uses the right variant from the pile of restored artifacts when they're all merged into one directory. I think this is enough to go on to start with; we'll just have to go from there on long-term viability.

## Considerations

### Variant Count

**Concern**: If each dependency has many variants, we'll waste bandwidth restoring unused artifacts.

**Mitigation strategies**:
- Monitor variant counts via telemetry/storage statistics
- Record usage and implement TTL for old variants (e.g., evict after 30 days unused)
- Add max variants per dependency (e.g., keep only 10 most recent)
- Use LRU eviction when cache size exceeds limits

**Hypothesis**: Most dependencies will have few variants because:
- Many crates don't have build scripts
- Many build scripts are relatively or fully deterministic
- System-dependent scripts probably mainly vary by feature and target triple, which are already part of the cache key; additional system variation (library paths, OS versions, etc.) likely produces few distinct artifacts within a given cache key.

### Storage Size

**Concern**: Build outputs (especially compiled C libraries) can be large.

**Mitigation strategies**:
- Monitor variant sizes via telemetry/storage statistics
- Compress aggressively; build outputs compress reasonably well
- Use content-addressable storage to deduplicate common files across variants

### `mtime` Handling

When restoring artifacts, cargo compares modification times.

Today in the proof-of-concept we just set consistent mtimes on all restored files, which is probably _fine_ although not _ideal_.

It might be better to figure out a "smart restore" strategy. Right now I think this might look like:
1. When backing up build artifact variants, record the files in the build that depend on them
2. When restoring build artifact variants, set their mtime to be the same or slightly before the dependant

For example:
- A library `rlib` depends on the build script being run (its output folder)
- The build script being run depends on the build script being built
- We can build a whole graph of this using Cargo's `unit-graph` functionality (we already use this for computing cache keys)
- We can then artificially reconstruct a set of `mtime`s that make sense to Cargo based on this graph.

## Future ideas

> [!NOTE]
> These may or may not be good ideas, we'll need to experiment.

### Restore Optimization

**Current approach**: Restore all variants for the dependency cache key unconditionally.

**Potential future approaches**:
- Qualified restores: Record information about the machine that added each variant and only restore ones that match along some axis
- Lazy restores: Restore variants using a cargo wrapper similar to `sccache`, use information about the build so far to guide restores

## Alternative Approaches Considered

### Input Fingerprinting via Static Analysis

**Idea**: Statically analyze build scripts to determine what inputs they depend on, then cache by input fingerprint.

**How it would work**:
1. Classify build scripts into categories:
   - No build script
   - "Const" (no external inputs)
   - "Pure" (deterministic outputs based on input)
   - "Effectful" (non-deterministic outputs based on inputs)
2. Compute appropriate cache key based on the inputs, and enumerate the list of inputs
3. Store each variant per dependency
4. On restore, find the variants for the dependency and evaluate the list of inputs for each one until a matching cache key is found

**Why we rejected it**:
1. We'd need to precompute, which means analyzing all of `crates.io` perpetually
2. Only works for pre-analyzed public crates
3. Requires static analysis, tracing (`strace`/`dtrace`), determinism testing, and who knows what else

And worst of all we have to do all the above before we can reasonably even validate if it'll work.

**When we might revisit this**:
- If we see too many variants per dependency, leading to too much wasted time/bandwidth/storage
- If the shotgun approach doesn't actually lead to many cache hits

For reference, here's a condensed version of my notes when researching this; hopefully they're a good starting point if we ever revisit.

1. Precomputation infrastructure:
  - Mirror `crates.io`
  - Build test harness to compile crates with different configurations
  - Run determinism tests (multiple machines, directories, timestamps)
  - Classify each crate version as no-script/const/pure/effectful

2. Tracing for input discovery:
  - Use `strace`/`dtrace` to capture file reads, env var access, etc
  - Store input lists per crate in database

3. Runtime classification:
  - For known crates, look up classification in database
  - For unknown crates, use conservative heuristics
  - Fall back to shotgun restore or always-rebuild

4. Maintenance:
  - Subscribe to crates.io RSS feed
  - Analyze new versions as they publish
