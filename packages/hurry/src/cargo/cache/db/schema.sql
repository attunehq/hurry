-- This schema is for the local SQLite file. Note that SQLite's behavior is
-- meaningfully different from Postgres's. In particular, it is weakly typed by
-- default, supports fewer datatypes, and foreign keys must be opted-in via
-- application pragma.
--
-- This schema file is kept up-to-date manually. It should be updated whenever a
-- new migration is created.
--
-- TODO: Automatically check and update this schema file from migrations in CI.
--
-- See also:
-- 1. Datatypes: https://www.sqlite.org/datatype3.html

-- Objects stored in the CAS.
CREATE TABLE object (
  id INTEGER PRIMARY KEY,
  -- The CAS key of the object.
  key TEXT NOT NULL
);

-- Semantic packages that have been cached. A _package_ is defined by its
-- registry identity, and is composed of a source, name, and version. For now,
-- we only support the `crates.io` registry as a source, which is why we don't
-- yet have separate columns describing the source.
CREATE TABLE package (
  id INTEGER PRIMARY KEY,
  -- The name of the package.
  name TEXT NOT NULL,
  -- The version of the package.
  version TEXT NOT NULL
);

-- A build of a package. Each package can be built in many different ways (e.g.
-- with different feature flags, targets, or rustc flags). A _build_ is a
-- cacheable and restoreable set of artifacts associated with a specific build
-- configuration of a package.
--
-- If a set of artifacts cannot be restored as-is given a build configuration,
-- then it should be a different `package_build` row (if two rows are otherwise
-- identical, then likely we are missing fields).
--
-- Note that this includes:
--
-- 1. The library crate of the package.
-- 2. The build script of the package, if one exists.
-- 3. The build script outputs of the package, if a build script exists.
CREATE TABLE package_build (
  id INTEGER PRIMARY KEY,
  package_id INTEGER NOT NULL REFERENCES package(id),

  -- The target identifier ("target triple") of the build.
  target TEXT NOT NULL,

  -- Release profiles parameters. These currently capture the parameters of the
  -- _library crate_ of the package, not of the build script.
  --
  -- TODO: These fields come from `cargo build` JSON messages, but the
  -- `.profile` field in the unit graph has more fields than this. Do those need
  -- to be included? Some of them (e.g. `lto`) look like they may not be
  -- specific to a package build.
  opt_level TEXT NOT NULL,
  debuginfo TEXT NOT NULL,
  debug_assertions BOOLEAN NOT NULL,
  overflow_checks BOOLEAN NOT NULL,
  test BOOLEAN NOT NULL,

  -- Build features. Note that SQLite does not support ARRAY types. Therefore,
  -- we store this as a JSON array of strings sorted in lexicographic order, so
  -- we can use string equality to rapidly query for the same features.
  features TEXT NOT NULL,

  -- The Rust compiler edition.
  edition TEXT NOT NULL,

  -- TODO: Do we need to add the Rust compiler version as a key? I think the
  -- answer is no, but `cargo build` does call `rustc -vV` on boot.

  -- This is the `-C extra-filename` flag passed to `rustc` in order to build
  -- the library crate of this package. This is computed by Cargo from elements
  -- of the built unit, such as package ID, features, optimization flags, rustc
  -- version, etc.[^1]. In theory, we don't need to store this, since the other
  -- fields we are using as keys should fully describe the inputs into this
  -- flag, but we retain this field as a sanity check just in case. If we ever
  -- see rows where this value is the same for different builds of the same
  -- package, but the other fields are different (or vice versa: if this value
  -- is different when other fields are the same), then we know that something
  -- about how we're keying artifacts is wrong.
  --
  -- [^1]: https://github.com/rust-lang/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/build_runner/compilation_files.rs#L631
  extra_filename TEXT NOT NULL

  -- TODO: We will need to add some fields to capture build script outputs,
  -- especially directives that should cause the cached artifacts to invalidate.
  -- This may include:
  --
  -- 1. The hash of paths specified by `cargo::rerun-if-changed`.
  -- 2. The environment variable keys and values specified by
  --    `cargo::rerun-if-env-changed`.
  -- 3. Other `cargo::` directives, especially ones that set `rustc` flags.
  -- 4. The flags of the full `rustc` invocation.
  --
  -- See also: https://github.com/attunehq/hurry/pull/55
);

-- A `package_build_dependency` is a crate `package_build` that is used as a
-- dependency in another crate `package_build` via the `--extern` flag passed to
-- `rustc`. These are required because different crate dependencies cause
-- different artifacts to be built.
--
-- Note that these do NOT represent native libraries brought in through `-L`.
CREATE TABLE package_build_dependency (
  dependency INTEGER NOT NULL REFERENCES package_build(id),
  dependent INTEGER NOT NULL REFERENCES package_build(id)
);

-- Files created by a build. This connects a package build to its cached
-- artifact files in the CAS.
CREATE TABLE package_build_artifact (
  package_build_id INTEGER NOT NULL REFERENCES package_build(id),
  object_id INTEGER NOT NULL REFERENCES object(id),

  -- The path of the artifact within the target directory.
  path TEXT NOT NULL,

  -- The mtime of the artifact.
  mtime INTEGER NOT NULL,

  -- Whether the artifact should have its executable permission bit set.
  executable BOOLEAN NOT NULL
);
