-- Rollback hashed API keys to plaintext
-- This will delete all existing hashed tokens

TRUNCATE api_key CASCADE;

ALTER TABLE api_key DROP COLUMN hash;
ALTER TABLE api_key ADD COLUMN content TEXT NOT NULL UNIQUE;
