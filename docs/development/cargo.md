# Cargo

> [!CAUTION]
> This document is _developer_ documentation. It may be incomplete, and may reflect internal implementation details that are subject to change or have already changed. Rely on this documentation at your own risk.

> [!CAUTION]
> As of the `v2` commit, these docs are largely a wishlist/planning document that may be subject to (potentially significant) changes. This warning will be removed once we've implemented and settled on approaches.

## Invocation Sketch

When `hurry cargo build` is run:

1. Compute the workspace identifier.
2. Gather information about the workspace, used to select the appropriate cache key using the workspace cache manifest (if one exists).
3. If an exact match cache key is found, restore its contents to the workspace.
4. If no exact match cache key is found, restore the closest match cache key to the workspace.
5. Run the build with `cargo`.
6. If the build succeeds and the cache was not an exact match, store the current state of the workspace into the CAS and create a new cache key reference for this state of the repository.

> [!TIP]
> Information about the workspace is gathered from many source but in general they are something like (but not necessarily limited to):
> - Platform and CPU architecture
> - `cargo`/`rustc` version
> - `cargo`/`rustc` invocation flags
> - Hashed content of files on disk
>
> Different facts about the workspace have different levels of precedence when it comes to selecting the "most similar" cache key in the event of a non-exact match. For example, it is _almost_ useless to restore the cache for a system that is an entirely different platform and architecture. We may still do it if we detect potential reuse such as e.g. `sqlx` or other macro invocations which may actually be reused across different platforms and architectures in some contexts.

## Storage

`hurry` stores a user-local cache at `~/.cache/hurry`. For Rust, the current layout of this cache is:

```
~/.cache/hurry/v1/cargo/
├── ws/
│   ├── <workspace_identifier>
│   │   ├── lock
│   │   ├── manifest.json
│   │   └── <cache_key>/
│   └── ...
└── cas/
    ├── <object_b3sum>
    └── ...
```

Each `<workspace_identifier>` uniquely identifies a "logical" workspace.
The idea is that when users are working with e.g. git submodules or other similar systems where the same workspace is checked out at different paths, `hurry` will treat them as the same workspace, allowing cache reuse. This identifier is platform and machine independent, allowing for cache reuse across machines (e.g. CI or different developer systems).

The `lock` file is used to ensure that the workspace is not modified while a single instance of `hurry` is running. This is a hack; we will probably eventually replace this with atomic filesystem operations.

The `manifest.json` contains information about the workspace such as the hashes of the source files, cache keys (and what they represent), and the target directory. The idea here is that you can't just blindly rely on a single cache key for reliable cross-platform and cross-machine builds. Instead, we record metadata about the cache keys so that if there's not an exact match `hurry` can find the closest match to restore. `hurry` is built with a "fail-closed" architecture, meaning that if it restores a cache that is slightly incorrect, `cargo` or `rustc` will simply rebuild the parts that are missing.

Finally, the `<cache_key>` directories contain the actual build artifacts inside a given cache; these are equivalent to the `target` directory in a workspace. Each directory is made up of symlinks from the `cas` directory so that common files are able to be shared across multiple cache keys instead of duplicating space on disk.

When `hurry` is run in a workspace and needs to restore the cache, its behavior differs by platform:
- If Copy-On-Write functionality is available on the file system, it prefers that.
- If OverlayFS is available, it falls back to that in order to simulate Copy-On-Write.
- Otherwise, it falls back to a simple symlink approach.
