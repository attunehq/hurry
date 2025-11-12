-- Change API key hash from TEXT to BYTEA for SHA256
-- This is destructive: all existing tokens will be invalidated

-- Delete all existing tokens
TRUNCATE api_key CASCADE;

-- Change hash column from TEXT to BYTEA
ALTER TABLE api_key ALTER COLUMN hash TYPE BYTEA;
