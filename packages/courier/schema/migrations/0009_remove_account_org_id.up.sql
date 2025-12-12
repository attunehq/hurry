-- Migration 0009: Remove account.organization_id column
--
-- This migration completes the transition to the organization_member model.
-- The account.organization_id column is legacy and no longer used by the application.
--
-- Steps:
-- 1. Ensure all accounts have corresponding organization_member entries
-- 2. Ensure all API keys have organization_id set (from account.organization_id)
-- 3. Drop the account.organization_id column

-- Step 1: Create organization_member entries for any accounts that don't have them
-- (This handles any legacy accounts that were created before the organization_member table)
INSERT INTO organization_member (organization_id, account_id, role_id)
SELECT a.organization_id, a.id, (SELECT id FROM organization_role WHERE name = 'admin')
FROM account a
WHERE a.organization_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM organization_member om
    WHERE om.account_id = a.id AND om.organization_id = a.organization_id
  );

-- Step 2: Set api_key.organization_id from account.organization_id for keys that don't have it set
UPDATE api_key
SET organization_id = account.organization_id
FROM account
WHERE api_key.account_id = account.id
  AND api_key.organization_id IS NULL
  AND account.organization_id IS NOT NULL;

-- Step 3: Drop the organization_id column from account
ALTER TABLE account DROP COLUMN organization_id;
