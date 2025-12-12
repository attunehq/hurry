-- Short-lived, single-use auth codes issued after OAuth callback.
-- These avoid returning session tokens directly in URLs.
CREATE TABLE oauth_exchange_code (
  id BIGSERIAL PRIMARY KEY,
  -- Store only a hash of the exchange code (like API keys/sessions), so DB
  -- leaks don't allow redeeming live auth codes.
  code_hash BYTEA NOT NULL UNIQUE,
  account_id BIGINT NOT NULL REFERENCES account(id),
  redirect_uri TEXT NOT NULL,
  -- Stored server-side; never trusted from the client.
  new_user BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  redeemed_at TIMESTAMPTZ
);

CREATE INDEX idx_oauth_exchange_code_expires ON oauth_exchange_code(expires_at);
