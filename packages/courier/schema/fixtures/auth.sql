-- Test data for authentication and authorization

-- Organizations (skip ID 1 which is reserved for "Default Organization")
INSERT INTO organization (id, name, created_at) VALUES
  (2, 'Acme Corp', '2024-01-01 00:00:00+00'),
  (3, 'Widget Inc', '2024-01-02 00:00:00+00'),
  (4, 'Test Org', '2024-01-03 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- Accounts (org IDs updated to match new organization IDs)
INSERT INTO account (id, organization_id, email, created_at) VALUES
  (1, 2, 'alice@acme.com', '2024-01-01 00:00:00+00'),
  (2, 2, 'bob@acme.com', '2024-01-01 00:00:00+00'),
  (3, 3, 'charlie@widget.com', '2024-01-02 00:00:00+00'),
  (4, 4, 'test@test.com', '2024-01-03 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- API Keys (using simple tokens for testing)
INSERT INTO api_key (id, account_id, content, created_at, accessed_at, revoked_at) VALUES
  (1, 1, 'acme-alice-token-001', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', NULL),
  (2, 2, 'acme-bob-token-001', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', NULL),
  (3, 3, 'widget-charlie-token-001', '2024-01-02 00:00:00+00', '2024-01-02 00:00:00+00', NULL),
  (4, 4, 'test-token-001', '2024-01-03 00:00:00+00', '2024-01-03 00:00:00+00', NULL),
  (5, 1, 'acme-alice-token-revoked', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', '2024-01-15 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- Reset sequences to avoid conflicts
SELECT setval('organization_id_seq', (SELECT MAX(id) FROM organization));
SELECT setval('account_id_seq', (SELECT MAX(id) FROM account));
SELECT setval('api_key_id_seq', (SELECT MAX(id) FROM api_key));
