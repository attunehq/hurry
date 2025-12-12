-- Restore NOT NULL constraint on expires_at.
-- Set any NULL values to 7 days from now before adding constraint.
UPDATE organization_invitation
SET expires_at = NOW() + INTERVAL '7 days'
WHERE expires_at IS NULL;

ALTER TABLE organization_invitation ALTER COLUMN expires_at SET NOT NULL;
