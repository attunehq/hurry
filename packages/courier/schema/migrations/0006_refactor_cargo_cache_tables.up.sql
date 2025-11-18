-- Drop old cargo cache tables and indexes
DROP INDEX IF EXISTS idx_cargo_library_unit_build_artifact_build_id;
DROP INDEX IF EXISTS idx_cargo_library_unit_build_artifact_object_id;

DROP TABLE IF EXISTS cargo_library_unit_build_artifact;
DROP TABLE IF EXISTS cargo_library_unit_build;
DROP TABLE IF EXISTS cargo_package;
DROP TABLE IF EXISTS cargo_object;

-- Create new cargo cache tables

-- Models `UnitPlanInfo`.
--
-- Within a given `Vec<SavedUnit>`, each entry has a `UnitPlanInfo` attached
-- and this value is _always_ the same across the entire plan.
CREATE TABLE cargo_unit_plan_info (
  id BIGSERIAL PRIMARY KEY,
  unit_hash TEXT NOT NULL,
  package_name TEXT NOT NULL,
  crate_name TEXT NOT NULL,
  target_arch TEXT,
  UNIQUE(
    unit_hash,
    package_name,
    crate_name,
    target_arch
  )
);

-- Stores `SavedUnit` instances.
--
-- We store these using JSONB encoding:
-- - Many of the inner types use relatively extensive heterogenous types which
--   are difficult to model well in SQL.
-- - We don't really ever need to compose a `SavedUnit` instance from smaller
--   components; they tend to have a lot of local data that can't really be
--   shared.
-- - We expect the amount of data duplication to be relatively low.
--
-- If any of these points end up false in the future we can explore normalizing
-- the data into tables or moving parts of this into the CAS or some other
-- strategy.
--
-- Important: We _do_ expect that any instance where the actual content of files
-- is stored in this type is replaced with the CAS key.
CREATE TABLE cargo_saved_unit (
  id BIGSERIAL PRIMARY KEY,
  hash TEXT NOT NULL REFERENCES cargo_unit_plan_info(unit_hash),
  data JSONB NOT NULL,
  UNIQUE(hash, data)
);
