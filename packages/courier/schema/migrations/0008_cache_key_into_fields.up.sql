TRUNCATE TABLE cargo_saved_unit;

ALTER TABLE cargo_saved_unit
  DROP COLUMN cache_key,
  ADD COLUMN unit_hash TEXT NOT NULL,
  ADD COLUMN unit_resolved_target TEXT NOT NULL,
  ADD COLUMN linux_glibc_version TEXT;

DROP INDEX IF EXISTS idx_cargo_saved_unit_org_key;

CREATE INDEX IF NOT EXISTS idx_cargo_saved_unit_org_key ON cargo_saved_unit(organization_id, unit_hash);
