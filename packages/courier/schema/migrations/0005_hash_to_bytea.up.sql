-- Change API key hash from TEXT to BYTEA for SHA256
-- This is destructive: all existing tokens will be invalidated

-- Delete all existing tokens
TRUNCATE api_key CASCADE;

-- Drop and recreate hash column as BYTEA
ALTER TABLE api_key DROP COLUMN hash;
ALTER TABLE api_key ADD COLUMN hash BYTEA NOT NULL UNIQUE;
