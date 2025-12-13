-- Reverse self-service signup tables
-- Drop in reverse order of creation to respect foreign key constraints

-- Drop audit logging
DROP INDEX idx_audit_log_created;
DROP INDEX idx_audit_log_org;
DROP INDEX idx_audit_log_account;
DROP TABLE audit_log;

-- Drop user sessions
DROP INDEX idx_session_expires;
DROP INDEX idx_session_account;
DROP TABLE user_session;

-- Drop OAuth exchange codes
DROP INDEX idx_oauth_exchange_code_expires;
DROP TABLE oauth_exchange_code;

-- Drop OAuth state
DROP INDEX idx_oauth_state_expires;
DROP TABLE oauth_state;

-- Drop invitation system
DROP TABLE invitation_redemption;
DROP INDEX idx_invitation_expires;
DROP INDEX idx_invitation_org;
DROP TABLE organization_invitation;

-- Restore organization_id to account BEFORE dropping organization_member
-- so we can backfill from it. Add as nullable first, backfill, then make NOT NULL.
ALTER TABLE account ADD COLUMN organization_id BIGINT REFERENCES organization(id);

-- Backfill account.organization_id from organization_member
-- Pick one org per account- this isn't perfect, but it's the best we can do
-- with this old data model.
UPDATE account
SET organization_id = om.organization_id
FROM (
  SELECT DISTINCT ON (account_id) account_id, organization_id
  FROM organization_member
  ORDER BY account_id, organization_id
) om
WHERE account.id = om.account_id;
ALTER TABLE account ALTER COLUMN organization_id SET NOT NULL;

-- Drop columns that are no longer used
ALTER TABLE api_key DROP COLUMN organization_id;
ALTER TABLE account DROP COLUMN name;
ALTER TABLE account DROP COLUMN disabled_at;

-- Restore unique constraint on email
ALTER TABLE account ADD CONSTRAINT account_email_key UNIQUE (email);

-- Drop organization membership
DROP INDEX idx_org_member_role;
DROP INDEX idx_org_member_account;
DROP TABLE organization_member;
DROP TABLE organization_role;

-- Drop GitHub identity linking
DROP TABLE github_identity;

-- Restore account table comment
COMMENT ON TABLE account IS 'Each distinct actor in the application is an "account"; this could be humans or it could be bots. In the case of bots, the "email" field is for where the person/team owning the bot can be reached.';
