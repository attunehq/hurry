# Cargo

> [!CAUTION]
> This document is _developer_ documentation. It may be incomplete, and may reflect internal implementation details that are subject to change or have already changed. Rely on this documentation at your own risk.

> [!CAUTION]
> As of the `v2` commit, these docs are largely a wishlist/planning document that may be subject to (potentially significant) changes. This warning will be removed once we've implemented and settled on approaches.

## Invocation Sketch

When `hurry cargo build` is run:

1. Compute the workspace identifier.
2. Gather information about the workspace, used to select the appropriate cache key using the workspace cache manifest (if one exists).

> [!NOTE]
> Information about the workspace is gathered from many sources but in general they are something like (but not necessarily limited to):
> - Platform and CPU architecture
> - `cargo`/`rustc` version
> - `cargo`/`rustc` invocation flags
> - Hashed content of files on disk
>
> Different facts about the workspace have different levels of precedence when it comes to selecting the "most similar" cache key in the event of a non-exact match. For example, it is _almost_ useless to restore the cache for a system that is an entirely different platform and architecture. We may still do it if we detect potential reuse such as e.g. `sqlx` or other macro invocations which may actually be reused across different platforms and architectures in some contexts.

3. If an exact match cache key is found, restore its contents to the workspace.
4. If no exact match cache key is found, restore the closest match cache key to the workspace.

> [!NOTE]
> Restoring the contents to the workspace in effect means "copy the contents of the cache directory to the `target/` directory in the workspace". However, this "copy" operation is done in a few different ways; reference the "Storage" section below for more information.

5. Run the build with `cargo`.
6. If the build succeeds and the cache was not an exact match, store the current state of the workspace into the CAS and create a new cache key reference for this state of the repository.

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

## Native Rust builds

In order for `hurry` to integrate with `cargo`/`rust`, we need to understand how their builds work. This section explains how they work to the best of our understanding. The intention is to provide multiple purposes:
- Demonstrate to users that we actually understand how `cargo` and `rustc` work.
  - Of course, as a corrolary, provide a place for other experts in this space to correct any misunderstandings we may have.
  - If you're this person, we'd love to hear from you!
- Provide a place for our core assumptions to be documented.
  - Since this is our understanding of the baseline, all design decisions made in the product are based on this understanding.
- Accelerate onboarding for new contributors by providing a centralized resource for learning about `cargo` and `rustc` builds.

References:
- [Cargo Book](https://doc.rust-lang.org/cargo/)
- [Rustc Dev Guide](https://rustc-dev-guide.rust-lang.org/)
- [Rust Compiler Documentation](https://doc.rust-lang.org/stable/nightly-rustc/index.html)
- [Compiler section of the Rust Project Forge](https://forge.rust-lang.org/compiler/index.html)

See also, although not strictly on topic:
- [Rust Book](https://doc.rust-lang.org/book/)
- [Rust Project Forge](https://forge.rust-lang.org/)
- [Std Dev Guide](https://std-dev-guide.rust-lang.org/)
- [Rust Nomicon](https://doc.rust-lang.org/stable/nomicon/)

### `target/` organization

> **TODO**
> Figure out the minimum version of `cargo` and `rustc` that `hurry` supports. To begin with it'll probably be whatever version uses the observed layout we're working with, but over time we may need to add support for changes made in older (or newer!) versions.

```
# Source: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/layout/index.html
#
# Some of the comments and content below have been altered from the original
# source to better reflect `hurry`-specific details or observed Rust compiler
# behavior in rustc `1.88.0-aarch64-apple-darwin`.

# Shared variables
#
# - `$pkgname`: The value of the `name` field in `Cargo.toml` for the crate.
# - `$targetname`: The name of the compilation target, e.g. the name of the
#   binary or library being built.
# - `$meta`: The "metadata unit id" used to uniquely identify a "unit" in the
#   build graph. A "unit" is an object that has enough information so that
#   cargo knows how to build it, e.g. a dependency.

# This is the root directory for all output, the top-level package
# places all of its output here.
target/

    # Cache of `rustc -Vv` output. Contains:
    # - rustc version, release, name, commit hash, commit date
    # - the host triple (e.g. `aarch64-apple-darwin`)
    # - the llvm version
    # - a bunch of details about the build environment, such as cpu features
    .rustc-info.json

    # Marks the `target/` directory as a cache directory.
    #
    # Currently ignored by `hurry`: this seems useless; the content is
    # actually static and set by this "Cache Directory Tagging Specification".
    # Reference: https://bford.info/cachedir/
    CACHEDIR.TAG

    # All final artifacts are linked into this directory from `deps`.
    # Note that named profiles will soon be included as separate directories
    # here. They have a restricted format, similar to Rust identifiers, so
    # Cargo-specific directories added in the future should use some prefix
    # like `.` to avoid name collisions.
    #
    # For `hurry` specifically, these are the bread and butter of the actual
    # whole `target/` directory.
    debug/  # or release/

        # File used to lock the directory to prevent multiple cargo processes
        # from using it at the same time.
        #
        # It turns out that `hurry` _also_ needs to lock the directory to
        # prevent multiple `hurry` processes from using it at the same time,
        # and to prevent `cargo` itself from using it while `hurry` is
        # backing up or restoring the directory. So `hurry` will actually
        # just reuse this file as well the same way `cargo` does.
        .cargo-lock

        # Holds all of the fingerprint files for all packages.
        #
        # See docs: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/fingerprint/index.html#fingerprint-files
        .fingerprint/

            # The package shows up three times with different `$meta` values.
            # This shape seems to be for the build of the crate.
            $pkgname-$meta/
                # The original docs said "_set_ of source filenames for this
                # package", but from my observation I only see _one_ file.
                # This is a binary, so it seems like this might be the compiled
                # binary for this dependency?
                #
                # TODO: Is this actually a set of names?
                # TODO: Is this the compiled binary of the dependency?
                dep-lib-$targetname

                # Timestamp when this package was last built.
                #
                # This file has no meaningful content; check the `mtime`
                # of this file for when the dependency was last built.
                invoked.timestamp

                # The fingerprint hash.
                #
                # TODO: what hash function is used?
                # TODO: what is the input to the hash function?
                lib-$targetname

                # Detailed information used for logging the reason why
                # something is being recompiled.
                #
                # This contains a bunch of information about the configuration
                # of the rust compiler at build time.
                #
                # TODO: How can we use this for caching/cache invalidation?
                lib-$targetname.json

            # The package shows up three times with different `$meta` values.
            # This shape seems to be for the output of the build script.
            $pkgname-$meta/

                # This contains a bunch of information about the configuration
                # of the rust compiler at build time.
                #
                # TODO: How can we use this for caching/cache invalidation?
                run-build-script-build-script-build.json

                # This file appears to contain a hash.
                #
                # TODO: Where is this hash generated? What does it mean?
                run-build-script-build-script-build

            # The package shows up three times with different `$meta` values.
            # This shape seems to be for the build of the build script.
            $pkgname-$meta/

                # This file appears to contain a hash.
                # TODO: Where is this hash generated? What does it mean?
                build-script-build-script-build

                # This contains a bunch of information about the configuration
                # of the rust compiler at build time.
                #
                # TODO: How can we use this for caching/cache invalidation?
                build-script-build-script-build.json

                # This file appears to be an object file.
                dep-build-script-build-script-build

                # Timestamp when this package was last built.
                #
                # This file has no meaningful content; check the `mtime`
                # of this file for when the dependency was last built.
                invoked.timestamp

        # This is the root directory for all rustc artifacts except build
        # scripts, examples, and test and bench executables. Almost every
        # artifact should have a metadata hash added to its filename to
        # prevent collisions. One notable exception is dynamic libraries.
        #
        # TODO: What do we do with dynamic libraries?
        # TODO: The original documentation explicitly stated directories,
        # but the observed behavior is flat files in this directory. What's
        # the deal here? When did this change, or was it ever correct?
        deps/

            # The `$pkgname-$meta` here appears to correspond to a directory
            # inside the `.fingerprint` directory.
            #
            # TODO: These "dep-info" files seem like they'll be really useful
            # for what we want to do with build caching.
            $pkgname-$meta.d

            # TODO: What is this? It appears to be an object file?
            # TODO: Where is this file generated?
            # TODO: How is `$???` generated?
            # TODO: Is the `0` significant?
            $pkgname-$meta.$pkgname.$???-cgu.0.rcgu.o

        # This is the location at which the output of all custom build
        # commands are rooted.
        build/

            # Each package gets its own directory where its build script output
            # and the output of building the build script are placed.
            #
            # The `$pkgname-$meta` here appears to correspond to a directory
            # inside the `.fingerprint` directory, but not the "Dep builds"
            # kind of directory inside `.fingerprint`- only the other two.
            #
            # TODO: The original doc says "build script names may be changed by
            # user"; what effect does this have on the contents of these
            # directories and the contents of the corresponding `.fingerprint`
            # directories?
            $pkgname-$meta/
                # The build script executable.
                #
                # TODO: The original doc says "name may be changed by user",
                # what does this mean? Would this file have a different name?
                build-script-build-$meta

                # Hard link to build-script-build-$meta.
                #
                # TODO: If the user changes the name of the build script,
                # what happens to this file?
                build-script-build

                # Dependency information generated by rustc.
                #
                # TODO: These "dep-info" files seem like they'll be really useful
                # for what we want to do with build caching.
                # TODO: If the user changes the name of the build script,
                # what happens to this file?
                build-script-build-$meta.d

            # The package shows up twice with two different metadata hashes.
            # This shape seems to be for the output of the build script.
            #
            # TODO: The original doc says "build script names may be changed by
            # user"; what effect does this have on the contents of these
            # directories and the contents of the corresponding `.fingerprint`
            # directories?
            $pkgname-$meta/
                # Timestamp when the build script was last executed.
                #
                # This file has no meaningful content; check the `mtime`
                # of this file for when the dependency was last built.
                invoked.timestamp

                # Directory where script can output files ($OUT_DIR).
                # This can be empty.
                out/

                # Output emitted by the build script when it was executed.
                #
                # Reference: https://doc.rust-lang.org/cargo/reference/build-scripts.html
                output

                # This file contains the fully qualified path to `out`.
                root-output

                # Stderr output from the build script.
                # This can be an empty file.
                stderr

        # Directory used to store incremental data for the compiler.
        #
        # TODO: This seems like a _great_ thing for `hurry` to reuse.
        incremental/

            # TODO: Where is `$pkgname` coming from?
            # TODO: How is `$???` generated?
            $pkgname-$???/

                # TODO: How are the `$???`'s generated?
                # TODO: Is the `s` significant?
                s-$???-$???.lock

                # The first two `$???` are the same as the corresponding lock.
                # TODO: How are the `$???`'s generated?
                s-$???-$???-$???/

                    # Contains a ton of seeming object files with hashes.
                    # TODO: How is `$???` generated?
                    $???.o

                    # TODO: What is this?
                    dep-graph.bin

                    # TODO: What is this?
                    query-cache.bin

                    # TODO: What is this?
                    work-products.bin

        # Root directory for all compiled examples.
        # Currently ignored by `hurry`: it appears low-value.
        examples/

    # Output from rustdoc.
    # Currently ignored by `hurry`: it appears low-value.
    doc/

    # Used by `cargo package` and `cargo publish` to build a `.crate` file.
    # Currently ignored by `hurry`: it appears low-value.
    package/

    # Experimental feature for generated build scripts.
    # Currently ignored by `hurry`: this is an experimental and niche feature.
    #
    # https://github.com/rust-lang/rfcs/pull/2196
    .metabuild/
```

### Fresh builds

> **TODO**
> What steps does `cargo` and `rustc` take to build a fresh workspace?

### Incremental third-party builds

> **TODO**
> What steps does `cargo` and `rustc` take to build incremental third-party crates?

> **TODO**
> My understanding today is that third-party crates are not actually built incrementally in the same way that first party crates are. Instead, they're always built fully, and an "incremental" third party build is really just "some of the third party dependencies have changed since the last build" (or similar- e.g. "the local rust compiler has changed" or "the rust flags have changed" also trigger rebuilds of third party crates). Is this correct?

### Incremental first-party builds

> **TODO**
> What steps does `cargo` and `rustc` take to build incremental first-party crates?

### Querying build information from Cargo

There are several ways to get build information out of Cargo:

1. Passing `RUSTC_WRAPPER` allows us to intercept and record calls to `rustc`.
2. `cargo metadata` provides information about the "packages" and "resolved dependencies" of the workspace.
3. `cargo build --message-format=json-diagnostic-rendered-ansi` provides formatted output messages from the compiler as it runs.
4. `cargo build --unit-graph` provides information about the "unit graph" of the build.
5. `cargo build --build-plan` provides information about the "build plan" of the build.

Each of these methods has their own trade-offs, and they're all incomplete or inconvenient in various ways. In order to get the information we need, we combine all of these methods.

#### `RUSTC_WRAPPER`

`RUSTC_WRAPPER` gives us the most detailed information, because we get the exact argv for each invocation of `rustc`.

This is nice because:

1. It includes all relevant flags, such as `-C extra-filename`, `--extern` flags, etc.
2. There are some values (like package version) that aren't available through flags, but are set through `CARGO_` environment variables defined [here](https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates).

Unfortunately:

1. These invocations only give us information at the level of `rustc`. There's no way for us to see from these invocations alone what the whole graph looks like, where the entrypoints to the graph are, or which invocations have dependencies on which other invocations (although we can try to infer dependency using `--extern` flags and the like).
2. There doesn't seem to be a way to infer which build scripts are for which packages.
3. `rustc` is not invoked when it's not needed! This means that during partial builds, not all invocations will be present!

#### `cargo metadata`

`cargo metadata`'s format is documented [here](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html#json-format).

This format provides us with a full list of workspace packages and the "resolved dependency graph".

Unfortunately, since this command isn't being invoked with build-time configuration (like `cargo build` flags), there's a lot of information it's missing:

1. It doesn't include the actual artifact filenames.
2. It can't tell which features are the ones actually being used in the build.
3. Consequently, it does not correctly resolve instances where a package and version are built multiple times with different features (e.g. how `openssl` uses `bitflags`).

#### `cargo build` JSON messages

`cargo build --message-format=json-diagnostic-rendered-ansi` provides formatted output messages from the compiler as it runs. This format is documented [here](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages).

Some nice properties of these messages:

1. They include the actual artifact filenames, including compiled build scripts.
2. They include parsed build script outputs.
3. They properly handle the actual build invocation flags (because they're an option of `cargo build`).
4. They include a `fresh` field, which indicates whether they were rebuilt or reused from cache.
5. They replay even when the unit is fresh and reused from cache.

Some not-so-nice properties of these messages:

1. They don't include the dependencies that go into each artifact! For example, `openssl` emits _two_ `compiler-artifact` messages for its library crate that have identical fields, because these artifacts are secretly being linked against different upstream dependencies.
2. The lack of dependencies also makes it hard to tell which build scripts are for which packages (for example, `semver` has two build scripts in `attune` because it's built twice with different features), although it can be somewhat guessed from message order.
3. These messages are surprisingly annoying to parse. In particular, doing so also turns off the normal human-friendly build messages, which actually changes observable behavior a bit.

#### `cargo build` unit graph

`cargo +nightly build --unit-graph -Z unstable-options` provides information about the "unit graph" of the build. This format is documented [here](https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph). Note that we can invoke this behavior on the stable Cargo tool by setting `RUSTC_BOOTSTRAP=1`.

Some nice properties of this format:

1. It includes the dependencies that go into each artifact! It even correctly distinguishes between units that seem identical to `cargo metadata` and the `cargo build` JSON output messages.
2. It properly handles the actual build invocation flags (because it's an option of `cargo build`).
3. It provides much more detail about optimization profile options.
4. It explicitly represents dependencies on build script compilation and build script execution (`run-custom-build`).
5. It provides the "roots" of the build (the top-level entrypoints), including when `cargo build` is invoked with specific targets.

Some not-so-nice properties of this format:

1. This format doesn't provide the actual artifact filenames, so we can't tell what which file suffix is used for artifacts of which unit.

#### `cargo build` build plan

`cargo +nightly build --build-plan -Z unstable-options` provides information about the "build plan" of the build. This format is documented [here](https://doc.rust-lang.org/cargo/reference/unstable.html#build-plan). Note that we can invoke this behavior on the stable Cargo tool by setting `RUSTC_BOOTSTRAP=1`.

Some nice properties of this format:

1. It provides the artifact names of all generated outputs.
2. It provides all `rustc` invocations _and_ all build script invocations.
3. It ties each invocation to the package being built.

Some not-so-nice properties of this format:

1. It doesn't provide a graph of dependencies (it only provides a list of invocations), so it doesn't tell us _why_ a particular invocation is being run.
2. The feature is allegedly deprecated, although it has not been removed in [six years](https://github.com/rust-lang/cargo/issues/7614).

#### How we combine this information

1. Use the unit graph to enumerate all units. In particular, this will provide us with the _dependencies_ of each unit, which we need in order to properly key the cached artifact and find its build script.
2. Use the build plan to map each unit to its generated artifacts (via `-C extra-filename`) and build script execution artifacts (via `OUT_DIR`).
   1. In cases where there are multiple matching units, each unit must either have (1) different features or (2) different dependencies. We can match features against the parsed invocation, and we can match dependencies by building a graph of `--extern` flags and mapping the upstream sources (which must have different features) to the unit graph.
   2. NOTE: the build plan invocations are slightly wrong because build plan construction does not run build scripts, and therefore can't know certain `rustc` arguments that can be added by build script outputs (e.g. `cargo::rustc-link-lib`). However, build script outputs cannot change `--extern` flags and cannot change `OUT_DIR`, so our usage of the build plan here is safe.
3. Use the `cargo build --message-format=json` `build-script-executed` messages to get the missing build script output flags, and map it to units via `out_dir`. We use this instead of `RUSTC_WRAPPER` because this saves and replays cached build script output of dependencies that don't need to be rebuilt, so we can get linker flags even for dependencies that are fresh.

Now, for each unit, we should know:
1. Its compiled artifact folder.
2. Its build script folder.
3. Its build script execution folder.
4. Its dependencies.
5. Its rustc flags.

This information should be sufficient to cache and key the unit.
