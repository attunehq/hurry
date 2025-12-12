-- Migration 0009 DOWN: Restore account.organization_id column
--
-- Note: This restore is lossy. If an account has multiple organization memberships,
-- only the first one (by organization_id) will be used for the restored column.
-- API keys retain their organization_id independently.

-- Step 1: Re-add the organization_id column (nullable initially)
ALTER TABLE account ADD COLUMN organization_id BIGINT REFERENCES organization(id);

-- Step 2: Populate from organization_member (using lowest org_id if multiple)
UPDATE account
SET organization_id = (
    SELECT om.organization_id
    FROM organization_member om
    WHERE om.account_id = account.id
    ORDER BY om.organization_id
    LIMIT 1
);

-- Step 3: For accounts with no organization_member, create a default org
-- This is a fallback - ideally all accounts should have memberships
INSERT INTO organization (name)
SELECT 'Default Organization for ' || a.email
FROM account a
WHERE a.organization_id IS NULL;

UPDATE account
SET organization_id = (
    SELECT o.id
    FROM organization o
    WHERE o.name = 'Default Organization for ' || account.email
    LIMIT 1
)
WHERE organization_id IS NULL;

-- Step 4: Make the column NOT NULL
ALTER TABLE account ALTER COLUMN organization_id SET NOT NULL;
