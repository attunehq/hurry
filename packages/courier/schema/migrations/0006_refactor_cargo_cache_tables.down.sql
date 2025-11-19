-- Drop new cargo cache tables
DROP TABLE IF EXISTS cargo_saved_unit;

-- Restore old cargo cache tables
CREATE TABLE cargo_object (
  id BIGSERIAL PRIMARY KEY,
  key TEXT NOT NULL,
  UNIQUE(key)
);

CREATE TABLE cargo_package (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  name TEXT NOT NULL,
  version TEXT NOT NULL,
  UNIQUE(organization_id, name, version)
);

CREATE TABLE cargo_library_unit_build (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  package_id BIGINT NOT NULL REFERENCES cargo_package(id),
  target TEXT NOT NULL,
  library_crate_compilation_unit_hash TEXT NOT NULL,
  build_script_compilation_unit_hash TEXT,
  build_script_execution_unit_hash TEXT,
  content_hash TEXT NOT NULL,
  UNIQUE NULLS NOT DISTINCT (organization_id, package_id, target, library_crate_compilation_unit_hash, build_script_compilation_unit_hash, build_script_execution_unit_hash)
);

CREATE TABLE cargo_library_unit_build_artifact (
  library_unit_build_id BIGINT NOT NULL REFERENCES cargo_library_unit_build(id),
  object_id BIGINT NOT NULL REFERENCES cargo_object(id),
  path TEXT NOT NULL,
  mtime NUMERIC(39, 0) NOT NULL,
  executable BOOLEAN NOT NULL,
  UNIQUE(library_unit_build_id, path)
);

CREATE INDEX idx_cargo_library_unit_build_artifact_build_id ON cargo_library_unit_build_artifact(library_unit_build_id);
CREATE INDEX idx_cargo_library_unit_build_artifact_object_id ON cargo_library_unit_build_artifact(object_id);
