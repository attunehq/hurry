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

-- Drop organization membership
DROP INDEX idx_org_member_role;
DROP INDEX idx_org_member_account;
DROP TABLE organization_member;
DROP TABLE organization_role;

-- Drop GitHub identity linking
DROP TABLE github_identity;

-- Remove columns from existing tables
ALTER TABLE api_key DROP COLUMN organization_id;
ALTER TABLE account DROP COLUMN name;
ALTER TABLE account DROP COLUMN disabled_at;

-- Restore organization_id column to account (required, will need data migration)
ALTER TABLE account ADD COLUMN organization_id BIGINT NOT NULL REFERENCES organization(id);
