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
--
-- Note: Organization membership is tracked via the organization_member table.
-- Accounts can belong to multiple organizations.
CREATE TABLE account (
  id BIGSERIAL PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  disabled_at TIMESTAMPTZ,
  name TEXT
);

-- Keys for accounts to use to authenticate.
CREATE TABLE api_key (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  name TEXT NOT NULL,
  hash BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  revoked_at TIMESTAMPTZ,
  organization_id BIGINT REFERENCES organization(id)
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

-- Links a GitHub user to their Courier account (1:1)
CREATE TABLE github_identity (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id) UNIQUE,
  github_user_id BIGINT NOT NULL UNIQUE,
  github_username TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Defines valid roles for organization membership
CREATE TABLE organization_role (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed initial roles
INSERT INTO organization_role (name, description) VALUES
  ('member', 'Regular organization member'),
  ('admin', 'Organization administrator with full permissions');

-- Tracks which accounts belong to which organizations
CREATE TABLE organization_member (
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  account_id BIGINT NOT NULL REFERENCES account(id),
  role_id BIGINT NOT NULL REFERENCES organization_role(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (organization_id, account_id)
);

CREATE INDEX idx_org_member_account ON organization_member(account_id);
CREATE INDEX idx_org_member_role ON organization_member(role_id);

-- Invitations for users to join organizations
CREATE TABLE organization_invitation (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  token TEXT NOT NULL UNIQUE,
  role_id BIGINT NOT NULL REFERENCES organization_role(id),
  created_by BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  max_uses INT,
  use_count INT NOT NULL DEFAULT 0,
  revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_invitation_org ON organization_invitation(organization_id);
CREATE INDEX idx_invitation_expires ON organization_invitation(expires_at);

-- Tracks who used which invitation
CREATE TABLE invitation_redemption (
  id BIGSERIAL PRIMARY KEY,
  invitation_id BIGINT NOT NULL REFERENCES organization_invitation(id),
  account_id BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (invitation_id, account_id)
);

-- Temporary storage for OAuth flow state
CREATE TABLE oauth_state (
  id BIGSERIAL PRIMARY KEY,
  state_token TEXT NOT NULL UNIQUE,
  pkce_verifier TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_oauth_state_expires ON oauth_state(expires_at);

-- Active user sessions (for web UI authentication)
CREATE TABLE user_session (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT NOT NULL REFERENCES account(id),
  session_token BYTEA NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_session_account ON user_session(account_id);
CREATE INDEX idx_session_expires ON user_session(expires_at);

-- Records authorization-related events
CREATE TABLE audit_log (
  id BIGSERIAL PRIMARY KEY,
  account_id BIGINT REFERENCES account(id),
  organization_id BIGINT REFERENCES organization(id),
  action TEXT NOT NULL,
  details JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_log_account ON audit_log(account_id);
CREATE INDEX idx_audit_log_org ON audit_log(organization_id);
CREATE INDEX idx_audit_log_created ON audit_log(created_at);
