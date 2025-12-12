-- Change session_token from TEXT to BYTEA for SHA256 hash storage
-- This is destructive: all existing sessions will be invalidated

-- Delete all existing sessions
TRUNCATE user_session CASCADE;

-- Drop and recreate session_token column as BYTEA
ALTER TABLE user_session DROP COLUMN session_token;
ALTER TABLE user_session ADD COLUMN session_token BYTEA NOT NULL UNIQUE;
