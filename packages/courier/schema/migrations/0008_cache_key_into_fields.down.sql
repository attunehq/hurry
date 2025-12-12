TRUNCATE cargo_saved_unit;

ALTER TABLE cargo_saved_unit
  DROP COLUMN unit_hash,
  DROP COLUMN unit_resolved_target,
  DROP COLUMN linux_glibc_version,
  ADD COLUMN cache_key TEXT NOT NULL;

DROP INDEX IF EXISTS idx_cargo_saved_unit_org_key;

CREATE INDEX IF NOT EXISTS idx_cargo_saved_unit_org_key ON cargo_saved_unit(organization_id, cache_key);
