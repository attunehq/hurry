INSERT INTO organization (id, name) VALUES (1, 'test-org-1');
INSERT INTO organization (id, name) VALUES (2, 'test-org-2');

INSERT INTO account (id, organization_id, email) VALUES (1, 1, 'account1@org1.example');
INSERT INTO account (id, organization_id, email) VALUES (2, 1, 'account2@org1.example');
INSERT INTO account (id, organization_id, email) VALUES (3, 2, 'account1@org2.example');
INSERT INTO account (id, organization_id, email) VALUES (4, 2, 'account2@org2.example');

INSERT INTO api_key (account_id, content) VALUES (1, 'test-api-key-account1-org1');
INSERT INTO api_key (account_id, content) VALUES (2, 'test-api-key-account2-org1');
INSERT INTO api_key (account_id, content) VALUES (3, 'test-api-key-account1-org2');
INSERT INTO api_key (account_id, content) VALUES (4, 'test-api-key-account2-org2');
