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

-- TODO: Actually we have to rethink this schema, because a package can be
-- included with different features. Maybe we should think about it in terms of
-- "lib crates"? But how do we tell which build scripts are for which libs?
-- Maybe by looking at features?
--
-- ANSWER: Actually, I think packages with different features should count as
-- different packages, and in each case we just have the build script and the
-- lib crate. I don't know what to do about the 3-target packages though.

-- TODO: Hmm, can we look at the OUT_DIR in the rustc invocations for the libs,
-- and map those back to the --out-dir set for the build script?

-- TODO: What about the packages with _3_ targets? How is that possible?
--
-- ANSWER: I think this is because it's linking to different dependencies, as we
-- can see in the rustc invocation. _Why_ is it linking to different
-- dependencies? I'm not sure - it shouldn't resolve that way because there's
-- only one inclusion in the cargo tree.
--
-- ANSWER: Oh, openssl is linked against the same version of bitflags, but that
-- version of bitflags has two builds with different feature sets I think. Why
-- does this cause linkage against both? Why not just link against the one that
-- gets used? Do they both get used?

-- TODO: Do the same packages have the same extra-filenames (i.e. hash
-- filepaths) between invocations in the same project? What about the same
-- package across different projects? I think the code suggests yes, but I
-- haven't actually empirically tested this yet.

-- Builds of packages. Each package can be built in many different ways (e.g.
-- with different feature flags, targets, or rustc flags). A _build_ is a
-- `rustc` invocation for a package that specifies these parameters.
--
-- TODO: Note that this is not quite sufficient to model the build. In
-- particular, we are capturing things like the profile parameters of "the"
-- build, when in reality there are separate `rustc` invocations for both the
-- build script and the library. As future work, we should separately capture
-- the Cargo invocation and configuration (e.g. Cargo.toml build configuration
-- and environment variables) and the associated `rustc` invocations.
CREATE TABLE package_build (
  id INTEGER PRIMARY KEY,
  package_id INTEGER NOT NULL REFERENCES package(id),

  -- The target identifier ("target triple") of the build.
  target TEXT NOT NULL,

  -- Release profiles parameters. These currently capture the parameters of the
  -- _library crate_ of the package, not of the build script.
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
);

-- TODO: We need to key the build on the dependencies
CREATE TABLE package_build_dependency (
  dependency INTEGER NOT NULL REFERENCES package_build(id),
  dependent INTEGER NOT NULL REFERENCES package_build(id)
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
