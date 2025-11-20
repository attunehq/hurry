-- Schema file for Courier.
--
-- After making changes to this file, create a migration in ./migrations to
-- apply the new changes. Each migration should be sequentially ordered after
-- the previous one using its numeric prefix.

-- Organizations in the instance.
CREATE TABLE organization (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Each distinct actor in the application is an "account"; this could be humans
-- or it could be bots. In the case of bots, the "email" field is for where the
-- person/team owning the bot can be reached.
CREATE TABLE account (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  email TEXT NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Keys for accounts to use to authenticate.
CREATE TABLE api_key (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  hash BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  revoked_at TIMESTAMPTZ
);

-- Lists CAS keys known about by the database.
--
-- Since the CAS keys are actually on disk, technically there could be keys
-- that exist that are not in the database (or vice versa) but the ones in the
-- database are the only ones that the application knows exist.
CREATE TABLE cas_key (
  id BIGSERIAL PRIMARY KEY,
  content BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Controls what organizations have access to a given CAS key.
--
-- We deduplicate CAS keys: if two organizations both save the same content,
-- we only actually store one copy of it (since they're keyed by content, they
-- are by defintion safe to deduplicate).
--
-- Organizations are given access after they upload the content themselves.
CREATE TABLE cas_access (
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  cas_key_id BIGINT NOT NULL REFERENCES cas_key(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (organization_id, cas_key_id)
);

-- Cargo cache: stores SavedUnit instances as JSONB.
--
-- This table uses a JSONB-based approach for simplicity and flexibility:
-- - SavedUnit types are directly serialized to JSONB without decomposition
-- - cache_key is a stable hash of SavedUnitCacheKey (includes unit hash + future fields)
-- - No impedance mismatch: Rust types ARE the storage format
-- - Future-proof: Adding fields to SavedUnitCacheKey doesn't require schema changes
--
-- Access pattern is simple key-value:
-- - Save: INSERT complete SavedUnit by cache_key
-- - Restore: SELECT by cache_key and deserialize
--
-- File contents are stored in CAS (deduplicated), JSONB only stores metadata.
CREATE TABLE cargo_saved_unit (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  cache_key TEXT NOT NULL,
  data JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(organization_id, cache_key)
);

CREATE INDEX idx_cargo_saved_unit_org_key ON cargo_saved_unit(organization_id, cache_key);
