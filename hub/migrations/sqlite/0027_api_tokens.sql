-- M1-07: Per-user API tokens for scripts, mobile app, PSA integrations
-- Bearer token format: blt_<prefix>_<secret>
-- Only secret_hash is stored; token shown once at creation

CREATE TABLE api_tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  prefix TEXT NOT NULL,                  -- 8 chars, URL-safe
  secret_hash TEXT NOT NULL,             -- SHA-256 of the full secret
  read_only INTEGER NOT NULL DEFAULT 0,  -- optional read-only flag
  created_at DATETIME NOT NULL,
  last_used_at DATETIME,
  expires_at DATETIME
);

CREATE INDEX idx_api_tokens_user_id ON api_tokens(user_id);
CREATE INDEX idx_api_tokens_prefix ON api_tokens(prefix);
