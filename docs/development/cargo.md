# Cargo

> [!CAUTION]
> This document is _developer_ documentation. It may be incomplete, and may reflect internal implementation details that are subject to change or have already changed. Rely on this documentation at your own risk.

`hurry` stores a user-local cache at `~/.cache/hurry`. For Rust, the current layout of this cache is:

```
~/.cache/hurry/v1/cargo/
├── workspaces/
│   ├── <workspace_path>
│   │   ├── meta.db
│   │   └── target/
│   └── ...
└── cas/
    ├── <object_b3sum>
    └── ...
```

Each `<workspace_path>` is the BLAKE3 hash of the absolute path to the workspace directory. When `hurry` is run in a workspace, it symlinks that workspace's `target` directory to `<workspace_path>/target/` so we can swap out the cache contents as needed.

`meta.db` is a SQLite database that stores metadata about each cached workspace. It contains the following tables:

- `invocation`, which tracks invocations of `hurry`.
  - `invocation_id`
  - `argv`
  - `start_time`
  - `end_time`
- `source_file`, which tracks source files in the workspace. Source files are uniquely identified by their BLAKE3 hash.
  - `source_file_id`
  - `b3sum`
- `invocation_source_file`, which tracks the source files that were used in an invocation.
  - `invocation_id`
  - `source_file_id`
  - `path`
  - `mtime`
- `artifact`, which tracks compiled artifacts stored in the CAS.
  - `artifact_id`
  - `b3sum`
- `invocation_artifact`, which tracks the artifacts that were emitted after an invocation.
  - `invocation_id`
  - `artifact_id`
  - `path`
  - `mtime`

## Branch-specific `target` caches



### Future work

You can imagine a version of this design that's git-agnostic, and works to restore caches whenever a previous cache could be valid.

Here's a sketch. When `hurry cargo build` is run:

1. Record a new invocation $I$.
2. Check every source file in the workspace to see whether its `(path, b3sum)` matches a file from a previous invocation. While doing so, record the source file (including its mtime) into $I$.
3. If every file has the same `(path, b3sum)` as some previous invocation $I_0$, then:
   1. Restore the mtimes of every source file to their recorded mtimes from before running build $I_0$.
   2. Restore the `target` cache from the CAS to the recorded state after running build $I_0$.
4. Shell out to `cargo build`.
5. Record every artifact in `target` into $I$, saving them into the CAS.
