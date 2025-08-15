# Design

`hurry` is a command line tool that provides drop-in integrations with your native build tools to provide really, really fast builds. It does this by stacking a variety of tool-specific tricks.

> [!CAUTION]
> As of the `v2` commit, these docs are largely a wishlist/planning document that may be subject to (potentially significant) changes. This warning will be removed once we've implemented and settled on approaches.

## Overarching goals

`hurry` intends to:
- Tightly interact with each build tool it supports.
- Improve performance when doing builds for projects of any size so long as the project has been built at least once on any machine.
- Make "simple" cases like switching git branches or worktrees effectively instant.

The "north stars" for the design of `hurry` are:
- **Correctness**: `hurry` doesn't write incorrect cache state, or if it does then it es so in a way that allows the build system to detect and transparently recover from valid state by rebuilding instead of compiling incorrect code.
- **Cross-everything**: `hurry` can restore caches across different platforms, architectures, compiler versions, git branches, git worktrees, etc.
- **Performant**: Obviously this is all a wash if we can't actually speed up builds significantly in the majority of cases.
- **Disk friendly**: `hurry` is designed to minimize disk usage as much as possible while still maximizing cache hit rates.
- **Daemonless**: `hurry` is designed to be a standalone tool that doesn't require a daemon to run.

If the north star goals conflict, they are resolved in the order written above.
All the details in this and other docs are meant to provide a comprehensive understanding of how `hurry` achieves these goals.

## Rust

For Rust, `hurry` integrates with `cargo` via `hurry cargo`. Here's how it works at a high level.

### `cargo build`

By default, code from within your workspace (as opposed to dependency code) is built _incrementally_. These incremental artifacts are cached in your `target/` directory.

Let's say you have two branches in your git repository, `A` and `B`. When you switch from `A` to `B`, do a little work (possibly running a `cargo build`), and then switch back to `A`, Cargo will not be able to reuse your incremental build cache for `A`!

Why is this? Two reasons:

1. If you did an incremental build while working in `B`, then your incrementally cached builds of the files while they were in branch `A` have been overwritten by the new build.
2. Even if you didn't do an incremental build, your build cache has been invalidated, because switching branches changed the mtime of your source files, and Cargo uses mtime to determine whether a file has been changed!

`hurry` works around this for you. When it finds that your workspace is in a previously compiled state, it restores your previous local incremental cache and the mtimes of your source files, so Cargo will reuse the cached artifacts.

> [!NOTE]
> The Rust project's first-party solution for this is to invalidate the fingerprint of compiled artifacts when the _contents_ of the compiled source files have changed. This progress is tracked in [rust-lang/cargo#14136](https://github.com/rust-lang/cargo/issues/14136).

### `cargo run`

`hurry` passes through all arguments provided on the command line to `cargo run`.

> [!WARNING]
> `hurry` does not currently accelerate compilation for `cargo run` invocations, although this is something we plan to add.

### Other commands

`hurry` does not currently support any other cargo command, although this is something we plan to add. Eventually `hurry` will support any arbitrary `cargo` command, even `cargo` plugins such as `cargo sqlx`, with support for accelerated builds.
