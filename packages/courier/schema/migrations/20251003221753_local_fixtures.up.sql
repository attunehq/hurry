insert into organizations (id, name) values (1, 'local');
insert into users (id, organization_id, email) values (1, 1, 'local@localhost.com');
insert into api_keys (id, user_id, content) values (1, 1, 'local');
