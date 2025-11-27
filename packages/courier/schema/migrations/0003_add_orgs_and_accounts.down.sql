-- Remove organization_id from cargo tables

ALTER TABLE cargo_library_unit_build
  DROP CONSTRAINT cargo_library_unit_build_org_pkg_target_hashes_key;

ALTER TABLE cargo_package
  DROP CONSTRAINT cargo_package_organization_id_name_version_key;

ALTER TABLE cargo_library_unit_build
  DROP COLUMN organization_id;

ALTER TABLE cargo_package
  DROP COLUMN organization_id;

-- Restore original unique constraints

ALTER TABLE cargo_package
  ADD CONSTRAINT cargo_package_name_version_key
  UNIQUE(name, version);

ALTER TABLE cargo_library_unit_build
  ADD CONSTRAINT cargo_library_unit_build_package_id_target_library_crate_co_key
  UNIQUE NULLS NOT DISTINCT (package_id, target, library_crate_compilation_unit_hash, build_script_compilation_unit_hash, build_script_execution_unit_hash);

-- Drop authentication and authorization tables

DROP TABLE cas_access;
DROP TABLE cas_key;
DROP TABLE api_key;
DROP TABLE account;
DROP TABLE organization;
