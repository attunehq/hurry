INSERT INTO organizations (id, name) VALUES (1, 'test-org-1');
INSERT INTO organizations (id, name) VALUES (2, 'test-org-2');

INSERT INTO users (id, organization_id, email) VALUES (1, 1, 'user1@test1.com');
INSERT INTO users (id, organization_id, email) VALUES (2, 1, 'user2@test1.com');
INSERT INTO users (id, organization_id, email) VALUES (3, 2, 'user1@test2.com');
INSERT INTO users (id, organization_id, email) VALUES (4, 2, 'user2@test2.com');

INSERT INTO api_keys (user_id, content) VALUES (1, 'test-token:user1@test1.com');
INSERT INTO api_keys (user_id, content) VALUES (2, 'test-token:user2@test1.com');
INSERT INTO api_keys (user_id, content) VALUES (3, 'test-token:user1@test2.com');
INSERT INTO api_keys (user_id, content) VALUES (4, 'test-token:user2@test2.com');
