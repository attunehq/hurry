-- Add cache tables for distributed caching

CREATE TABLE cargo_object (
  id BIGSERIAL PRIMARY KEY,
  key TEXT NOT NULL,
  UNIQUE(key)
);

CREATE TABLE cargo_package (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  version TEXT NOT NULL,
  UNIQUE(name, version)
);

CREATE TABLE cargo_library_unit_build (
  id BIGSERIAL PRIMARY KEY,
  package_id BIGINT NOT NULL REFERENCES cargo_package(id),
  target TEXT NOT NULL,
  library_crate_compilation_unit_hash TEXT NOT NULL,
  build_script_compilation_unit_hash TEXT,
  build_script_execution_unit_hash TEXT,
  content_hash TEXT NOT NULL,
  UNIQUE NULLS NOT DISTINCT (package_id, target, library_crate_compilation_unit_hash, build_script_compilation_unit_hash, build_script_execution_unit_hash)
);

CREATE TABLE cargo_library_unit_build_artifact (
  library_unit_build_id BIGINT NOT NULL REFERENCES cargo_library_unit_build(id),
  object_id BIGINT NOT NULL REFERENCES cargo_object(id),
  path TEXT NOT NULL,
  mtime NUMERIC(39, 0) NOT NULL,
  executable BOOLEAN NOT NULL,
  UNIQUE(library_unit_build_id, path)
);
