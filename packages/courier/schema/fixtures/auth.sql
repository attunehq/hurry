-- Test data for authentication and authorization

-- Organizations (skip ID 1 which is reserved for "Default Organization")
INSERT INTO organization (id, name, created_at) VALUES
  (2, 'Acme Corp', '2024-01-01 00:00:00+00'),
  (3, 'Widget Inc', '2024-01-02 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- Accounts (organization membership now tracked via organization_member table)
INSERT INTO account (id, email, created_at) VALUES
  (1, 'alice@acme.com', '2024-01-01 00:00:00+00'),
  (2, 'bob@acme.com', '2024-01-01 00:00:00+00'),
  (3, 'charlie@widget.com', '2024-01-02 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- Organization memberships (Alice and Bob in Acme, Charlie in Widget)
INSERT INTO organization_member (organization_id, account_id, role_id)
SELECT 2, 1, id FROM organization_role WHERE name = 'admin'
ON CONFLICT DO NOTHING;
INSERT INTO organization_member (organization_id, account_id, role_id)
SELECT 2, 2, id FROM organization_role WHERE name = 'member'
ON CONFLICT DO NOTHING;
INSERT INTO organization_member (organization_id, account_id, role_id)
SELECT 3, 3, id FROM organization_role WHERE name = 'admin'
ON CONFLICT DO NOTHING;

-- API Keys (using SHA256 hashed tokens stored as BYTEA)
-- Token plaintext values:
--   acme-alice-token-001 -> fd69f4bc9b3cef00adeba62feedd4cbe0eaca7bf4875bff5bad544b4d96cde33
--   acme-bob-token-001 -> 80a4f0c064a7eb3f3aaf126bba6f74cab42e40717c865173f9fde9b46b54cc2a
--   widget-charlie-token-001 -> 8daf0ecd9b51b1b1bc57c34530e6406dd554afa6c4d11ee730aca8a498021254
--   acme-alice-token-revoked -> 209e0f7fe399a496966090284536e81018df959c1a3fafef0c819c2a08d571f0
INSERT INTO api_key (id, account_id, name, hash, organization_id, created_at, accessed_at, revoked_at) VALUES
  (1, 1, 'alice-primary', decode('fd69f4bc9b3cef00adeba62feedd4cbe0eaca7bf4875bff5bad544b4d96cde33', 'hex'), 2, '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', NULL),
  (2, 2, 'bob-primary', decode('80a4f0c064a7eb3f3aaf126bba6f74cab42e40717c865173f9fde9b46b54cc2a', 'hex'), 2, '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', NULL),
  (3, 3, 'charlie-primary', decode('8daf0ecd9b51b1b1bc57c34530e6406dd554afa6c4d11ee730aca8a498021254', 'hex'), 3, '2024-01-02 00:00:00+00', '2024-01-02 00:00:00+00', NULL),
  (4, 1, 'alice-revoked', decode('209e0f7fe399a496966090284536e81018df959c1a3fafef0c819c2a08d571f0', 'hex'), 2, '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', '2024-01-15 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- GitHub identities (so these accounts are recognized as human, not bot accounts)
INSERT INTO github_identity (id, account_id, github_user_id, github_username, created_at, updated_at) VALUES
  (1, 1, 1001, 'alice-github', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00'),
  (2, 2, 1002, 'bob-github', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00'),
  (3, 3, 1003, 'charlie-github', '2024-01-02 00:00:00+00', '2024-01-02 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- Reset sequences to avoid conflicts
SELECT setval('organization_id_seq', (SELECT MAX(id) FROM organization));
SELECT setval('account_id_seq', (SELECT MAX(id) FROM account));
SELECT setval('api_key_id_seq', (SELECT MAX(id) FROM api_key));
SELECT setval('github_identity_id_seq', (SELECT MAX(id) FROM github_identity));
