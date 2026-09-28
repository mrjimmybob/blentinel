-- Probes as records, added by enrollment codes (ROADMAP M2-01, proposal 0002,
-- D-47). Replaces the `[[probes]]` whitelist in blentinel_hub.toml. The hub
-- decides each probe's company and site (D-17); `probe_heartbeats` stays the
-- liveness table.

CREATE TABLE probes (
    id            TEXT     PRIMARY KEY,   -- the probe ID: hex of its Ed25519 public key
    name          TEXT     NOT NULL,
    company_id    TEXT     NOT NULL REFERENCES companies(id),
    site_id       TEXT     NOT NULL,
    state         TEXT     NOT NULL,      -- active | revoked
    hostname      TEXT,
    os            TEXT,
    arch          TEXT,
    probe_version TEXT,
    enrolled_at   DATETIME NOT NULL,
    revoked_at    DATETIME,
    token_id      TEXT,                   -- the code it enrolled with
    FOREIGN KEY (company_id, site_id) REFERENCES sites(company_id, id)
);

CREATE INDEX idx_probes_company ON probes(company_id);

-- "Add probe": one code, for one or several enrollments, until it expires.
CREATE TABLE enrollment_tokens (
    id          TEXT     PRIMARY KEY,     -- hex of 8 random bytes (also in the code)
    secret_hash TEXT     NOT NULL,        -- SHA-256 of the code's secret; the secret isn't stored
    company_id  TEXT     NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    site_id     TEXT     NOT NULL,
    probe_name  TEXT,                     -- NULL: the probe's hostname
    created_by  TEXT,                     -- username
    created_at  DATETIME NOT NULL,
    expires_at  DATETIME NOT NULL,
    max_uses    INTEGER  NOT NULL DEFAULT 1,
    uses        INTEGER  NOT NULL DEFAULT 0,
    revoked_at  DATETIME
);
