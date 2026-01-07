# Root Cause Analysis: Dependency Fingerprint Hash Not Found

## Issue

GitHub Issue: #319

User reports `dependency fingerprint hash not found` error when restoring cache:

```
Error:
   0: dependency fingerprint hash not found

Location:
   packages/hurry/src/cargo/fingerprint.rs:221

  SPANTRACE:
   0: hurry::cargo::fingerprint::rewrite with path=Some("/home/yonas/.cargo/registry/src/.../clap_derive-4.5.49/src/lib.rs")
```

## Reproduction

This reproduction requires access to a pre-populated cache that exhibits the bug.

### Prerequisites

1. Provide API credentials for accessing the `repro/319` org (ID: 46) in Hurry production
2. Docker with buildx support
3. The user's test project: https://github.com/yonasBSD/github-rs (branch `chore/rust-stable`, commit `3eeaef5b27d4167f857ab6574065250066ba300e`)

### Steps

```bash
# Set your API token for the repro/319 org
export HURRY_API_TOKEN="<token-for-repro-319-org>"

# Run the reproduction script
./packages/e2e/repros/319/repro.sh
```

The script will:
1. Clone the test project (github-rs)
2. Run hurry in a Docker container matching the user's environment (x86_64 Linux, Rust 1.92)
3. Attempt to restore from the cloned cache, triggering the bug

### Expected Failure

```
Error:
   0: dependency fingerprint hash not found

Location:
   packages/hurry/src/cargo/fingerprint.rs:221

  SPANTRACE:
   0: hurry::cargo::fingerprint::rewrite with path=Some("/.../crossbeam-utils-0.8.21/src/lib.rs")
```

## Root Cause

### The Problem

When restoring from cache, the server can return a unit **without returning all of its dependencies**. When this happens, fingerprint rewriting fails because the dependency's fingerprint hash mapping is never recorded.

### Detailed Flow

1. **Client requests restore** for N units (e.g., 328 units)
2. **Server returns fewer units** (e.g., 93 units) - many filtered out for various reasons:
   - glibc version incompatibility
   - Units not uploaded originally
   - Cache eviction
3. **For returned units**, hurry attempts to rewrite fingerprints
4. **Fingerprint rewriting** requires looking up dependency fingerprint hashes in a map (`dep_fingerprints`)
5. **The dependency is missing** because its unit was not returned by the cache API
6. **Error**: `dep_fingerprints.get(&old_dep_fingerprint)` returns `None`

### Concrete Example

From the reproduction logs:

```
# Server returned crossbeam-utils LibraryCrate but NOT its build script dependency
queuing unit restore: LibraryCrate { package_name: "crossbeam-utils", deps: [109] }
unit missing from cache response: BuildScriptCompilation, pkg_name: crossbeam-utils
unit missing from cache response: BuildScriptExecution, pkg_name: crossbeam-utils

# When rewriting fingerprint for LibraryCrate, it looks for dep 109's fingerprint
rewrite fingerprint deps: start
rewriting fingerprint dep: DepFingerprint { name: "build_script_build", memoized_hash: Some(12399637767299727662) }
# ERROR: hash 12399637767299727662 not in dep_fingerprints map!
```

### Code Location

**Error site** (`packages/hurry/src/cargo/fingerprint.rs:219-221`):
```rust
dep.fingerprint = dep_fingerprints
    .get(&old_dep_fingerprint)
    .ok_or_eyre("dependency fingerprint hash not found")?  // <-- fails here
    .clone();
```

**Missing handling** (`packages/hurry/src/cargo/cache/restore.rs:231-254`):
```rust
let Some(saved) = saved_units.take(&unit_hash.into()) else {
    // Unit missing from cache - we skip it but DON'T record anything
    // for dep_fingerprints, even though dependent units might need it
    debug!("unit missing from cache response");
    progress.dec_length(1);
    continue;  // <-- returns to caller with incomplete dep_fingerprints
};
```

### Why the Assumption is Violated

The code implicitly assumes: **if a unit is returned by cache, all its dependencies are either:**
1. Also returned by cache, OR
2. Already present on disk (skipped units)

This assumption fails when:
- Server filters out dependencies (e.g., glibc incompatibility) but returns dependents
- Partial cache uploads leave gaps in the dependency chain
- Cache eviction removes some units but not others

## Investigation Data

### Cache Response Statistics

```
requested_count: 328
returned_count: 93
```

Only 28% of requested units were returned, causing many dependency chain breaks.

### Key Missing Units

Build script units were frequently filtered out while their dependent library crates were returned:
- `crossbeam-utils` BuildScriptCompilation - MISSING
- `crossbeam-utils` BuildScriptExecution - MISSING
- `crossbeam-utils` LibraryCrate - RETURNED (depends on above)

### Debug Logs

The full debug logs from reproduction are available in `logs.tar.gz`.

To view:
```bash
tar -xzf logs.tar.gz
less logs/hurry-debug.log
```

Key search patterns:
```bash
grep "unit missing from cache" logs/hurry-debug.log
grep "dependency fingerprint hash not found" logs/hurry-debug.log
grep "crossbeam-utils" logs/hurry-debug.log
```

## References

### Hurry Source
- `packages/hurry/src/cargo/fingerprint.rs` - Fingerprint rewriting logic
- `packages/hurry/src/cargo/cache/restore.rs` - Cache restore orchestration

### Related Issues
- Issue #319: Original user report
