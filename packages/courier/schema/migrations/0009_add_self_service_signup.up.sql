-- Add self-service signup tables for GitHub OAuth authentication
-- See RFC docs/rfc/0003-self-service-signup.md for details

-- Add disabled_at and name to account
ALTER TABLE account ADD COLUMN disabled_at TIMESTAMPTZ;
ALTER TABLE account ADD COLUMN name TEXT;

-- Remove unique constraint on email to allow bot accounts to share emails
-- (bot accounts use the owner's contact email, which may duplicate human accounts)
ALTER TABLE account DROP CONSTRAINT account_email_key;

-- Update account comment
COMMENT ON TABLE account IS 'Each distinct actor in the application is an "account"; this could be humans or it could be bots. In the case of bots, the "email" field is for where the person/team owning the bot can be reached. Note: Organization membership is tracked via the organization_member table. Accounts can belong to multiple organizations.';

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

-- Backfill organization_member from account.organization_id
-- Existing accounts become admins of their organizations becuase we didn't have
-- roles before this migration.
INSERT INTO organization_member (organization_id, account_id, role_id)
SELECT account.organization_id, account.id, organization_role.id
FROM account, organization_role
WHERE account.organization_id IS NOT NULL
  AND organization_role.name = 'admin';

-- Add organization_id to api_key (all keys are org-scoped)
-- First add as nullable, backfill from account.organization_id, then make NOT NULL
ALTER TABLE api_key ADD COLUMN organization_id BIGINT REFERENCES organization(id);
UPDATE api_key SET organization_id = account.organization_id FROM account WHERE api_key.account_id = account.id;
ALTER TABLE api_key ALTER COLUMN organization_id SET NOT NULL;

-- Now we can remove organization_id from account (membership tracked via organization_member)
ALTER TABLE account DROP COLUMN organization_id;

-- Invitations for users to join organizations
CREATE TABLE organization_invitation (
  id BIGSERIAL PRIMARY KEY,
  organization_id BIGINT NOT NULL REFERENCES organization(id),
  token TEXT NOT NULL UNIQUE,
  role_id BIGINT NOT NULL REFERENCES organization_role(id),
  created_by BIGINT NOT NULL REFERENCES account(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ,                -- NULL = never expires
  max_uses INT,                          -- NULL = unlimited
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

-- Short-lived, single-use auth codes issued after OAuth callback.
-- These avoid returning session tokens directly in URLs.
CREATE TABLE oauth_exchange_code (
  id BIGSERIAL PRIMARY KEY,
  -- Store only a hash of the exchange code (like API keys/sessions), so DB
  -- leaks don't allow redeeming live auth codes.
  code_hash BYTEA NOT NULL UNIQUE,
  account_id BIGINT NOT NULL REFERENCES account(id),
  redirect_uri TEXT NOT NULL,
  new_user BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  redeemed_at TIMESTAMPTZ
);

CREATE INDEX idx_oauth_exchange_code_expires ON oauth_exchange_code(expires_at);

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
