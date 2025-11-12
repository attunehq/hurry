-- Schema file for Courier.
-- This file is maintained by hand; we use `sql-schema` to generate migrations.
--
-- After making changes to this file, run `sql-schema` to generate a migration
-- within the root of the `courier` package:
-- ```
-- sql-schema migration --name {new name here}
-- ```

-- Authentication and Authorization

CREATE TABLE organization (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE account (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  email TEXT NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE api_key (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  hash BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  revoked_at TIMESTAMPTZ
);

-- Content-Addressed Storage Access Control

CREATE TABLE cas_key (
  id BIGSERIAL PRIMARY KEY,
  content BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE cas_access (
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  cas_key_id BIGINT NOT NULL REFERENCES cas_key(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (organization_id, cas_key_id)
);

-- Cargo Build Artifacts (Global, Deduplicated)

CREATE TABLE cargo_object (
  id BIGSERIAL PRIMARY KEY,
  key TEXT NOT NULL,
  UNIQUE(key)
);

-- Cargo Cache Metadata (Org-Namespaced)

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
