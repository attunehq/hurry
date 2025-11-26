-- Add name field to api_key table
-- For existing rows, we provide a temporary default during migration
ALTER TABLE api_key ADD COLUMN name TEXT NOT NULL DEFAULT 'migration-default';

-- Remove default immediately so future inserts require explicit names
ALTER TABLE api_key ALTER COLUMN name DROP DEFAULT;
