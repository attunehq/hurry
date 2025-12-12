-- Allow organization_invitation.expires_at to be NULL (never expires)
ALTER TABLE organization_invitation ALTER COLUMN expires_at DROP NOT NULL;
