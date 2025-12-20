# Root Cause Analysis: Ring Build Failure (Path Doubling)

## Reproduction

The `Dockerfile` in this directory reproduces the bug with a minimal setup:

1. **Build hurry locally** with the buggy code (before this fix)
2. **Run `docker build`** which:
   - Copies the hurry binary into a fresh Rust container
   - Copies the courier source code
   - Runs `hurry cargo build --release --bin courier`
3. **Hurry restores cached artifacts** including build script outputs for crates like `ring`
4. **Cargo reads the restored `root-output` file** which contains a relative path (the bug)
5. **Cargo's path rewriting corrupts the `-L` flags** causing link failures

The reproduction requires cached build artifacts in Courier that include build script outputs. The bug manifests when restoring these artifacts in a different build environment (Docker container) where Cargo attempts to rewrite paths.

The reproduction script at `./repro.sh` automates these steps for you.

## Symptom

When building courier in Docker with hurry cache restore, the ring crate fails to link:

```
error: could not find native static library `ring_core_0_17_14_`, perhaps an -L flag is missing?
```

The rustc command shows a malformed `-L` flag with a doubled path:

```
-L native=/courier/target/release//courier/target/release/build/ring-e7f88aa1fb4f9250/out
```

Note the `//` in the middle - the path `/courier/target/release/` appears twice.

## Root Cause

In the buggy version (before this fix), the issue was located in `packages/hurry/src/cargo/cache/restore.rs` around lines 594-600.

Hurry writes a **relative** path to the `root-output` file, but Cargo expects an **absolute** path.

### The Problematic Code

```rust
// Generate root-output file (not from CAS - synthesized from unit_plan).
let root_output_path = profile_dir.join(&unit_plan.root_output_file()?);
fs::write(
    &root_output_path,
    unit_plan.out_dir()?.as_os_str().as_encoded_bytes(),  // <-- BUG: relative path
)
.await?;
```

`unit_plan.out_dir()` returns `build/ring-e7f88aa1fb4f9250/out` (a relative path), but Cargo expects the absolute `$OUT_DIR` that was in effect when the build script ran.

### How Cargo Uses root-output

When Cargo loads a "fresh" build script result, it reads `root-output` to determine the original `$OUT_DIR`. This is used to rewrite paths in the build script's `output` file when the target directory has moved (e.g., moving a project to a different location, using `--target-dir` to override the default, or CI systems that build in different paths than development machines).

From `cargo/src/cargo/core/compiler/custom_build.rs`:

```rust
// Lines 1359-1361: Read the previous OUT_DIR from root-output
let prev_script_out_dir = paths::read_bytes(&root_output_file)
    .and_then(|bytes| paths::bytes2path(&bytes))
    .unwrap_or_else(|_| script_out_dir.clone());

// Lines 925-928: Replace old paths with new paths in build script output
let value = value.replace(
    script_out_dir_when_generated.to_str().unwrap(),
    script_out_dir.to_str().unwrap(),
);
```

### The Path Doubling Mechanism

1. Hurry restores the `output` file with the reconstructed absolute path:
   ```
   cargo:rustc-link-search=native=/courier/target/release/build/ring-e7f88aa1fb4f9250/out
   ```

2. Hurry writes a **relative** path to `root-output`:
   ```
   build/ring-e7f88aa1fb4f9250/out
   ```

3. Cargo reads `root-output` and sets `script_out_dir_when_generated` to `build/ring-e7f88aa1fb4f9250/out`

4. Cargo's current `script_out_dir` is `/courier/target/release/build/ring-e7f88aa1fb4f9250/out` (absolute)

5. Cargo performs a string replacement on the `output` file contents:
   - Find: `build/ring-e7f88aa1fb4f9250/out` (relative)
   - Replace with: `/courier/target/release/build/ring-e7f88aa1fb4f9250/out` (absolute)

6. **The key problem**: The relative path `build/ring-e7f88aa1fb4f9250/out` is a *substring* of the absolute path `/courier/target/release/build/ring-e7f88aa1fb4f9250/out`. Cargo's `.replace()` finds this substring in the middle of the already-correct absolute path:
   ```
   native=/courier/target/release/build/ring-e7f88aa1fb4f9250/out
                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                                  ^-- substring match happens here
   ```

7. The replacement produces a corrupted doubled path:
   ```
   native=/courier/target/release//courier/target/release/build/ring-e7f88aa1fb4f9250/out
   ```

## Fix

Write the absolute path to `root-output`:

```rust
// Generate root-output file (not from CAS - synthesized from unit_plan).
let root_output_path = profile_dir.join(&unit_plan.root_output_file()?);
let out_dir_absolute = profile_dir.join(&unit_plan.out_dir()?);
fs::write(
    &root_output_path,
    out_dir_absolute.as_os_str().as_encoded_bytes(),
)
.await?;
```

This ensures that when Cargo performs its path replacement:
- If paths match (same machine/directory): replacement is a no-op
- If paths differ (moved target dir): replacement correctly updates the old absolute path to the new one

## Appendix: Why wasn't the `ring` rlib restored?

The logs show that the build script compilation and execution were restored from cache, but the library crate (`ring`) was missing:

```
restoring build script compilation unit, pkg_name: ring, unit_hash: 85cf8d57b86064e2
restoring build script OUT_DIR file, pkg_name: ring, unit_hash: e7f88aa1fb4f9250
...
unit missing from cache, unit_hash: UnitHash("f490a77d7d289d1d"), unit_type: LibraryCrate, pkg_name: ring
```

A natural follow-up question arises after investigating this bug: if we restored the build script outputs for `ring`, why wasn't the `ring` rlib itself restored? This section addresses that question.

### The build script restoration was correct

Hurry caches units by their `unit_id` hash, which Cargo computes in `compute_metadata()` (see `cargo/src/cargo/core/compiler/build_runner/compilation_files.rs:679-826`). This hash includes features, profile, and target architecture for *all* unit types, including build script execution. So the restored build script output was the correct one for this build configuration, not a "promiscuous" match from a different feature set.

### Why the rlib was missing: unknown

We cannot determine why the library crate was not in the cache with available information. Plausible explanations include:

- Transient upload issues: The library was never uploaded but the other units were; this would likely be due to network failure, timeouts, build interruption during cache save, etc.
- Download issues: Courier incorrectly claimed the library was missing when it actually exists; this seems very unlikely as we were able to replicate over several runs but is technically possible.
- Cross-compilation: The cache was populated by a `x86_64-unknown-linux-gnu` build that cross-compiled to a different target, so the host-targeted build scripts matched and restored correctly while the target-specific library was cached under a different `unit_id`, causing it to not be found for this build.
- There could also be a bug in our build script restoration logic; perhaps these build script units _should not have_ restored.

## References

### Cargo source
- `src/cargo/core/compiler/custom_build.rs`
  - `prev_build_output()` function reads `root-output`
  - `BuildOutput::parse()` performs the path replacement

### Hurry source
- `packages/hurry/src/cargo/cache/restore.rs`
  - Lines 594-600 write `root-output`

### Reproduction logs

The logs are compressed in `logs.tar.gz`. To extract and view them:

```bash
cd packages/hurry/repro/path-doubling
tar -xzf logs.tar.gz
```

This will extract the following directories:
- `run-local-repro/` - Logs from reproducing the issue
  - `2_docker_build.log` - Shows the doubled path in rustc invocation (search for `-L native=`)
  - `6_errors.log` - Filtered error messages including the link failure
- `run-local-fixed/` - Logs after applying the fix (path doubling resolved)
- `run-ci-1/`, `run-ci-2/` - Original CI failure logs from PR #294
