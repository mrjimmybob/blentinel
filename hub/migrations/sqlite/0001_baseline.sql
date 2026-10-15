-- Baseline schema (M0-05). Starts from scratch: pre-alpha databases created
-- before migrations existed are not adopted (DECISIONS.md D-30).
--
-- Rules for this folder (ENGINEERING.md §2.6):
--   * never edit a migration after it has been committed; add a new one
--   * file name = <version>_<description>.sql, versions strictly increasing
--   * connection settings (WAL, foreign keys, busy timeout) live in db::connect,
--     not here, because SQLite applies them per connection

-- ---------------------------------------------------------------------------
-- Probe reports
-- ---------------------------------------------------------------------------

CREATE TABLE reports (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    probe_id         TEXT     NOT NULL,
    company_id       TEXT     NOT NULL,
    hostname         TEXT     NOT NULL,
    site             TEXT     NOT NULL,
    timestamp        DATETIME NOT NULL,
    interval_seconds INTEGER  NOT NULL,
    received_at      DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- (probe_id, timestamp) serves both "reports of a probe" and the replay check
-- "latest report of a probe" (D-28).
CREATE INDEX idx_reports_probe_timestamp ON reports(probe_id, timestamp);
CREATE INDEX idx_reports_company_id      ON reports(company_id);
CREATE INDEX idx_reports_timestamp       ON reports(timestamp);

CREATE TABLE resource_statuses (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    report_id     INTEGER NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    resource_type TEXT    NOT NULL,
    target        TEXT    NOT NULL,
    status        TEXT    NOT NULL, -- 'Up' or 'Down'
    message       TEXT,
    latency_ms    INTEGER,
    metric_value  REAL,
    metric_unit   TEXT
);

CREATE INDEX idx_resource_statuses_report_id ON resource_statuses(report_id);

-- ---------------------------------------------------------------------------
-- Probe liveness: one row per known probe. Updated on every accepted report;
-- a background task flips status to 'expired' when last_seen_at falls behind.
-- ---------------------------------------------------------------------------

CREATE TABLE probe_heartbeats (
    probe_id      TEXT     PRIMARY KEY,
    company_id    TEXT     NOT NULL,
    status        TEXT     NOT NULL DEFAULT 'active',
    last_seen_at  DATETIME NOT NULL,
    first_seen_at DATETIME NOT NULL
);

CREATE INDEX idx_heartbeats_company ON probe_heartbeats(company_id);

-- ---------------------------------------------------------------------------
-- Archives
-- ---------------------------------------------------------------------------

CREATE TABLE archives (
    id          INTEGER  PRIMARY KEY AUTOINCREMENT,
    filename    TEXT     NOT NULL UNIQUE,
    created_at  DATETIME NOT NULL,
    cutoff_date DATETIME NOT NULL,
    size_mb     INTEGER  NOT NULL
);

CREATE INDEX idx_archives_created ON archives(created_at);

-- ---------------------------------------------------------------------------
-- Alerting
-- ---------------------------------------------------------------------------

CREATE TABLE alert_states (
    resource_key       TEXT PRIMARY KEY,
    last_status        TEXT NOT NULL,
    last_alert_sent_at DATETIME
);

CREATE TABLE alert_silences (
    id         INTEGER  PRIMARY KEY AUTOINCREMENT,
    scope_type TEXT     NOT NULL,
    scope_id   TEXT     NOT NULL,
    reason     TEXT     NOT NULL,
    created_at DATETIME NOT NULL,
    expires_at DATETIME,
    UNIQUE(scope_type, scope_id)
);

CREATE INDEX idx_silences_expires ON alert_silences(expires_at);
