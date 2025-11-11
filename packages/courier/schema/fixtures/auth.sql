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

-- API Keys (using SHA256 hashed tokens)
-- Token plaintext values:
--   acme-alice-token-001 -> fd69f4bc9b3cef00adeba62feedd4cbe0eaca7bf4875bff5bad544b4d96cde33
--   acme-bob-token-001 -> 80a4f0c064a7eb3f3aaf126bba6f74cab42e40717c865173f9fde9b46b54cc2a
--   widget-charlie-token-001 -> 8daf0ecd9b51b1b1bc57c34530e6406dd554afa6c4d11ee730aca8a498021254
--   test-token-001 -> 8fb2a50c5e3a3d2b09708fb12f65962def56a6e201cfe0d6d0ae7ad59fcf6af4
--   acme-alice-token-revoked -> 209e0f7fe399a496966090284536e81018df959c1a3fafef0c819c2a08d571f0
INSERT INTO api_key (id, account_id, hash, created_at, accessed_at, revoked_at) VALUES
  (1, 1, 'fd69f4bc9b3cef00adeba62feedd4cbe0eaca7bf4875bff5bad544b4d96cde33', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', NULL),
  (2, 2, '80a4f0c064a7eb3f3aaf126bba6f74cab42e40717c865173f9fde9b46b54cc2a', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', NULL),
  (3, 3, '8daf0ecd9b51b1b1bc57c34530e6406dd554afa6c4d11ee730aca8a498021254', '2024-01-02 00:00:00+00', '2024-01-02 00:00:00+00', NULL),
  (4, 4, '8fb2a50c5e3a3d2b09708fb12f65962def56a6e201cfe0d6d0ae7ad59fcf6af4', '2024-01-03 00:00:00+00', '2024-01-03 00:00:00+00', NULL),
  (5, 1, '209e0f7fe399a496966090284536e81018df959c1a3fafef0c819c2a08d571f0', '2024-01-01 00:00:00+00', '2024-01-01 00:00:00+00', '2024-01-15 00:00:00+00')
ON CONFLICT (id) DO NOTHING;

-- Reset sequences to avoid conflicts
SELECT setval('organization_id_seq', (SELECT MAX(id) FROM organization));
SELECT setval('account_id_seq', (SELECT MAX(id) FROM account));
SELECT setval('api_key_id_seq', (SELECT MAX(id) FROM api_key));
