-- Revert session_token from BYTEA to TEXT
-- This is destructive: all existing sessions will be invalidated

-- Delete all existing sessions
TRUNCATE user_session CASCADE;

-- Drop and recreate session_token column as TEXT
ALTER TABLE user_session DROP COLUMN session_token;
ALTER TABLE user_session ADD COLUMN session_token TEXT NOT NULL UNIQUE;
