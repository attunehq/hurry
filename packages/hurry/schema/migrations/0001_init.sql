-- Objects stored in the CAS.
CREATE TABLE object (
  id INTEGER PRIMARY KEY,
  -- The CAS key of the object.
  key TEXT NOT NULL UNIQUE
);

-- A Cargo package.
--
-- Packages are a Cargo concept. They are defined by their registry identity,
-- and are composed of a source, name, and version.
--
-- For now, we only support the `crates.io` registry as a source, which is why
-- we don't yet have separate columns describing the source.
CREATE TABLE package (
  id INTEGER PRIMARY KEY,
  -- The name of the package.
  name TEXT NOT NULL,
  -- The version of the package.
  version TEXT NOT NULL,
  UNIQUE(name, version)
);

-- A _build_ of a package's _library unit_.
--
-- Units are a Cargo concept. A unit represents a single piece of work. In
-- particular, Cargo uses units to represent each invocation of a program (i.e.
-- single `rustc` invocations, and compiled build script invocations).
--
-- Library units are a Hurry concept. We use the term "library unit of a
-- package" to refer to the following units in a package:
-- 1. The compilation of the package's library crate.
-- 2. The compilation of the package's build script, if one exists.
-- 3. The execution of the package's build script, if one exists.
--
-- Each package can be built in many different ways (e.g. with different
-- feature flags, targets, or rustc flags). A _build_ is a cacheable and
-- restoreable set of artifacts associated with a specific build configuration
-- of a package.
--
-- If a set of artifacts cannot be restored as-is given a build configuration,
-- then it should be a different `package_build` row (if two rows are otherwise
-- identical, then likely we are missing fields).
CREATE TABLE library_unit_build (
  id INTEGER PRIMARY KEY,
  package_id INTEGER NOT NULL REFERENCES package(id),

  -- The target identifier ("target triple") of the build.
  target TEXT NOT NULL,

  -- The unit hashes of each unit in the library unit. For compilation, these
  -- unit hashes are the same values passed to `-C extra-filename` when invoking
  -- `rustc`. For build script execution, this unit hash is used in the file
  -- path of the OUT_DIR.
  --
  -- In Cargo's source code, these are calculated here[^1].
  --
  -- [^1]: https://github.com/rust-lang/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/build_runner/compilation_files.rs#L616-L767
  library_crate_compilation_unit_hash TEXT NOT NULL,
  build_script_compilation_unit_hash TEXT,
  build_script_execution_unit_hash TEXT,

  -- The content hash of the library unit uniquely identifies the file contents
  -- being cached. We use this as error detection to determine when we've
  -- already cached a library unit build with different contents but the same
  -- key. This should never happen.
  content_hash TEXT NOT NULL

  -- TODO: There are other fields that we may want to include in keying the
  -- build. These are divided into _planned_ fields (i.e. known statically at
  -- plan-time) and _dynamic_ fields (i.e. not known until the build script is
  -- compiled and executed).
  --
  -- Planned fields include:
  -- 1. Compilation mode flags like opt_level, debuginfo, test, etc.
  -- 2. Features.
  -- 3. Compiler edition.
  -- 4. Resolved dependencies of this library unit instance.
  --
  -- Dynamic fields include:
  -- 1. Build script output directives.
  -- 2. Values specified by directives (e.g. hashes of files specified by
  --    `cargo::rerun-if-changed`, and keys/values of
  --    `cargo::rerun-if-env-changed`).
  -- 3. Full `rustc` invocation argv, which requires build directives to
  --    construct.
  --
  -- See also: https://github.com/attunehq/hurry/pull/55
);

-- Files created by a library unit build. This connects a library unit build to
-- its cached artifact files in the CAS.
CREATE TABLE library_unit_build_artifact (
  library_unit_build_id INTEGER NOT NULL REFERENCES library_unit_build(id),
  object_id INTEGER NOT NULL REFERENCES object(id),

  -- The path of the artifact within the target directory.
  path TEXT NOT NULL,

  -- The mtime of the artifact.
  --
  -- This is a Unix timestamp with nanosecond resolution. We can't store this in
  -- a native INTEGER, because INTEGERs only fit up to 8 bytes, and SQLite does
  -- not have BIGINTEGERs.
  mtime BLOB NOT NULL,

  -- Whether the artifact should have its executable permission bit set.
  executable BOOLEAN NOT NULL,

  UNIQUE(library_unit_build_id, path)
);
