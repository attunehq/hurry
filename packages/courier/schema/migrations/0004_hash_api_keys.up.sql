-- Destructive migration: hash API keys with sha256
-- All existing tokens will be invalidated and must be regenerated

-- Delete all existing tokens
TRUNCATE api_key CASCADE;

-- Drop old column and add new columns
ALTER TABLE api_key DROP COLUMN content;
ALTER TABLE api_key ADD COLUMN hash TEXT NOT NULL UNIQUE;
