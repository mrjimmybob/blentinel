-- Users, their company scope, and login sessions (ROADMAP M1-01, proposal 0001).
-- Replaces the shared admin token (D-06).

CREATE TABLE users (
    id                   TEXT     PRIMARY KEY,           -- random 128-bit hex
    username             TEXT     NOT NULL UNIQUE COLLATE NOCASE,
    display_name         TEXT     NOT NULL,
    email                TEXT,
    password_hash        TEXT     NOT NULL,              -- argon2id PHC string
    role                 TEXT     NOT NULL,              -- owner|admin|technician|viewer|client
    all_companies        INTEGER  NOT NULL DEFAULT 0,
    is_active            INTEGER  NOT NULL DEFAULT 1,
    must_change_password INTEGER  NOT NULL DEFAULT 0,
    created_at           DATETIME NOT NULL,
    last_login_at        DATETIME
);

-- Companies a user may see when all_companies = 0 (enforced from M1-04).
CREATE TABLE user_companies (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    company_id TEXT NOT NULL,
    PRIMARY KEY (user_id, company_id)
);

-- One row per login. The cookie holds a random value; only its SHA-256 is
-- stored, so a copy of the database can't be used to take over a session.
CREATE TABLE sessions (
    id_hash      TEXT     PRIMARY KEY,
    user_id      TEXT     NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   DATETIME NOT NULL,
    last_seen_at DATETIME NOT NULL,                      -- idle expiry is measured from here
    expires_at   DATETIME NOT NULL                       -- absolute expiry
);

CREATE INDEX idx_sessions_user ON sessions(user_id);
