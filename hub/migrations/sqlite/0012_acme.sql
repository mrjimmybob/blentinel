-- The hub's certificate from an ACME authority such as Let's Encrypt
-- (ROADMAP M2-14, proposal 0004 §4.4, D-62). The certificate and the ACME
-- account's key are files; the database holds the settings set on the
-- admin page and how the last attempt went.

-- No row: off (the hub serves a self-signed certificate).
CREATE TABLE acme_settings (
    id            INTEGER  PRIMARY KEY CHECK (id = 1),
    enabled       INTEGER  NOT NULL,
    domains       TEXT     NOT NULL,       -- JSON array of DNS names
    email         TEXT     NOT NULL,       -- '' : no contact
    authority     TEXT     NOT NULL,       -- lets_encrypt | lets_encrypt_staging | custom
    directory_url TEXT     NOT NULL,       -- custom only
    ca_pem        TEXT     NOT NULL,       -- custom only: its root certificate, if not public
    challenge     TEXT     NOT NULL,       -- tls_alpn | http
    terms_agreed  INTEGER  NOT NULL,
    updated_by    TEXT,                    -- username
    updated_at    DATETIME NOT NULL
);

-- The last time the hub asked the authority for a certificate.
CREATE TABLE acme_status (
    id              INTEGER  PRIMARY KEY CHECK (id = 1),
    last_attempt_at DATETIME NOT NULL,
    last_error      TEXT,                  -- NULL: it worked
    failures        INTEGER  NOT NULL,     -- failed attempts in a row (retries back off)
    last_issued_at  DATETIME
);
