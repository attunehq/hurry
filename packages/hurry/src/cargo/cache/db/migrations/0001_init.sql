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

-- Builds of packages. Each package can be built in many different ways (e.g.
-- with different feature flags, targets, or rustc flags). A _build_ is a
-- `rustc` invocation for a package that specifies these parameters.
CREATE TABLE package_build (
  id INTEGER PRIMARY KEY,
  package_id INTEGER NOT NULL REFERENCES package(id),

  -- The target identifier ("target triple") of the build.
  target TEXT NOT NULL,

  -- Release profiles parameters.
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
  edition TEXT NOT NULL
);

-- Files created by a build. This connects a package build to its cached
-- generated files in the CAS.
CREATE TABLE package_build_artifact (
  package_build_id INTEGER NOT NULL REFERENCES package_build(id),
  object_id INTEGER NOT NULL REFERENCES object(id),

  -- The path of the artifact within the target directory.
  path TEXT NOT NULL,

  -- Whether the artifact should have its executable permission bit set.
  executable BOOLEAN NOT NULL
);
