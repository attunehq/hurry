-- Rollback hash column from BYTEA to TEXT
-- This will delete all existing tokens

TRUNCATE api_key CASCADE;

ALTER TABLE api_key ALTER COLUMN hash TYPE TEXT;
