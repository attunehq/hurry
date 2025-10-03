CREATE TABLE organization (
  id bigserial PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  created TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE "user" (
  id bigserial PRIMARY KEY NOT NULL,
  organization_id BIGINT REFERENCES organization (id) NOT NULL,
  email TEXT NOT NULL UNIQUE,
  created TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_key (
  id bigserial PRIMARY KEY NOT NULL,
  user_id BIGINT REFERENCES "user" (id) NOT NULL,
  content BYTEA NOT NULL,
  created TIMESTAMPTZ NOT NULL DEFAULT now(),
  accessed TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked TIMESTAMPTZ,
  UNIQUE (content)
);

CREATE TABLE cas_key (
  id bigserial PRIMARY KEY NOT NULL,
  content BYTEA NOT NULL,
  created TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (content)
);

CREATE TABLE cas_access (
  org_id BIGINT REFERENCES organization (id) NOT NULL,
  cas_key_id BIGINT REFERENCES cas_key (id) NOT NULL,
  created TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id, cas_key_id)
);

CREATE TABLE frequency_user_cas_key (
  user_id BIGINT REFERENCES "user" (id) NOT NULL,
  cas_key_id BIGINT REFERENCES cas_key (id) NOT NULL,
  accessed TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, cas_key_id, accessed)
);

CREATE INDEX idx_frequency_user_key_recent ON frequency_user_cas_key(user_id, cas_key_id, accessed DESC);