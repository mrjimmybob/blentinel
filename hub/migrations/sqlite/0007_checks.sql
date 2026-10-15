-- Checks as hub records (ROADMAP M2-04, proposal 0003 §4.1, D-15, D-49).
-- The hub, not a file on the probe's machine, defines what each probe checks,
-- and sends it to the probe in its answer to a report. Results, alert state
-- and silences are keyed by the check's stable ID instead of the
-- `company:probe:name:target` text. Pre-alpha: earlier results, alert state
-- and resource silences are dropped rather than converted (D-30).

CREATE TABLE checks (
    id         TEXT     PRIMARY KEY,          -- CheckId: 16 random bytes, hex
    probe_id   TEXT     NOT NULL REFERENCES probes(id) ON DELETE CASCADE,
    name       TEXT     NOT NULL,
    kind       TEXT     NOT NULL,             -- ping | http | tcp | cpu | memory | disk | uptime
    spec       TEXT     NOT NULL,             -- common::checks::CheckSpec as JSON (kind included)
    enabled    INTEGER  NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    updated_by TEXT                           -- username
);

CREATE INDEX idx_checks_probe ON checks(probe_id);

-- What each probe runs (D-49). The hub works out the config a probe should
-- run from its checks and interval; these columns hold what the probe says.
ALTER TABLE probes ADD COLUMN interval_secs INTEGER NOT NULL DEFAULT 60;
ALTER TABLE probes ADD COLUMN config_revision TEXT;          -- the revision it runs
ALTER TABLE probes ADD COLUMN config_rejected_revision TEXT; -- the last one it refused
ALTER TABLE probes ADD COLUMN config_error TEXT;             -- and why

-- One row per check result in a report (replaces resource_statuses).
DROP TABLE resource_statuses;

CREATE TABLE check_results (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    report_id  INTEGER NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
    check_id   TEXT    NOT NULL,              -- no foreign key: history outlives a deleted check
    status     TEXT    NOT NULL,              -- 'Up' or 'Down'
    message    TEXT,
    latency_ms INTEGER
);

CREATE INDEX idx_check_results_report ON check_results(report_id);
CREATE INDEX idx_check_results_check  ON check_results(check_id);

-- Any number of values per result (proposal 0006 §1.4), in the order reported.
CREATE TABLE check_metrics (
    result_id INTEGER NOT NULL REFERENCES check_results(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    key       TEXT    NOT NULL,
    value     REAL    NOT NULL,
    unit      TEXT    NOT NULL,
    PRIMARY KEY (result_id, position)
);

-- Reports without results would count as probes with no checks.
DELETE FROM reports;

-- Alert state and silences of the old text keys can't match any check.
DELETE FROM alert_states WHERE resource_key NOT LIKE 'probe_expiry:%';
DELETE FROM alert_silences WHERE scope_type = 'resource';
