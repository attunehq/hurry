-- Rollback hash column from BYTEA to TEXT
-- This will delete all existing tokens

TRUNCATE api_key CASCADE;

ALTER TABLE api_key DROP COLUMN hash;
ALTER TABLE api_key ADD COLUMN hash TEXT NOT NULL UNIQUE;
