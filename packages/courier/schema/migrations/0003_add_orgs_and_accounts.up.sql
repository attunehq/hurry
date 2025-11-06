-- Add authentication and authorization tables

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
  content TEXT NOT NULL UNIQUE,
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

-- Create default organization for existing data
INSERT INTO organization (id, name, created_at)
VALUES (1, 'Default Organization', NOW())
ON CONFLICT (id) DO NOTHING;

-- Ensure the sequence is set correctly
SELECT setval('organization_id_seq', (SELECT MAX(id) FROM organization), true);

-- Add organization_id to cargo tables for multi-tenancy

ALTER TABLE cargo_package
  ADD COLUMN organization_id BIGINT REFERENCES organization(id);

ALTER TABLE cargo_library_unit_build
  ADD COLUMN organization_id BIGINT REFERENCES organization(id);

-- Assign all existing data to the default organization
UPDATE cargo_package SET organization_id = 1 WHERE organization_id IS NULL;
UPDATE cargo_library_unit_build SET organization_id = 1 WHERE organization_id IS NULL;

-- Update unique constraints to include organization_id
-- First drop the old constraints

ALTER TABLE cargo_package
  DROP CONSTRAINT cargo_package_name_version_key;

ALTER TABLE cargo_library_unit_build
  DROP CONSTRAINT cargo_library_unit_build_package_id_target_library_crate_co_key;

-- Add new constraints with organization_id

ALTER TABLE cargo_package
  ADD CONSTRAINT cargo_package_organization_id_name_version_key
  UNIQUE(organization_id, name, version);

ALTER TABLE cargo_library_unit_build
  ADD CONSTRAINT cargo_library_unit_build_org_pkg_target_hashes_key
  UNIQUE NULLS NOT DISTINCT (organization_id, package_id, target, library_crate_compilation_unit_hash, build_script_compilation_unit_hash, build_script_execution_unit_hash);

-- Make organization_id NOT NULL after adding constraints
-- (Assuming no existing data, or existing data should be deleted)

ALTER TABLE cargo_package
  ALTER COLUMN organization_id SET NOT NULL;

ALTER TABLE cargo_library_unit_build
  ALTER COLUMN organization_id SET NOT NULL;
