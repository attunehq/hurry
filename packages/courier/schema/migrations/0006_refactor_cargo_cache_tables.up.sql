-- Drop old cargo cache tables and indexes
DROP INDEX IF EXISTS idx_cargo_library_unit_build_artifact_build_id;
DROP INDEX IF EXISTS idx_cargo_library_unit_build_artifact_object_id;

DROP TABLE IF EXISTS cargo_library_unit_build_artifact;
DROP TABLE IF EXISTS cargo_library_unit_build;
DROP TABLE IF EXISTS cargo_package;
DROP TABLE IF EXISTS cargo_object;

-- Create new JSONB-based cargo cache table
--
-- This table stores SavedUnit instances directly as JSONB:
-- - cache_key: stable hash of SavedUnitCacheKey (unit hash + future fields)
-- - data: complete SavedUnit serialized to JSONB
-- - No decomposition, no reconstruction complexity
-- - Future-proof: cache key can evolve without schema changes
CREATE TABLE cargo_saved_unit (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  cache_key TEXT NOT NULL,
  data JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(organization_id, cache_key)
);

CREATE INDEX idx_cargo_saved_unit_org_key ON cargo_saved_unit(organization_id, cache_key);
