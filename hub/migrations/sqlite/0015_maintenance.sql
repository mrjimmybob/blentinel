-- Maintenance windows and silences of any scope (ROADMAP M3-04, proposal
-- 0005 §2.4, D-65).

-- A site's IANA time zone: its maintenance windows follow its local time.
ALTER TABLE sites ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';

-- Planned work: no alerts for the scope while open, and its results don't
-- count for uptime.
CREATE TABLE maintenance_windows (
    id         INTEGER  PRIMARY KEY AUTOINCREMENT,
    company_id TEXT     NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    scope_kind TEXT     NOT NULL,      -- company | site | probe | check
    scope_id   TEXT,                   -- NULL for the whole company
    reason     TEXT     NOT NULL,
    timezone   TEXT     NOT NULL,      -- IANA zone of the schedule's times
    schedule   TEXT     NOT NULL,      -- model::Schedule as JSON
    created_by TEXT,
    created_at DATETIME NOT NULL
);

CREATE INDEX idx_maintenance_windows_company ON maintenance_windows(company_id);

-- Silences now cover a check, a probe, a site or a whole company, always
-- within one company. The table is made again for its new key; existing
-- check silences keep their company (their probe's).
CREATE TABLE alert_silences_new (
    id         INTEGER  PRIMARY KEY AUTOINCREMENT,
    company_id TEXT     NOT NULL,
    scope_type TEXT     NOT NULL,      -- company | site | probe | check
    scope_id   TEXT     NOT NULL,      -- '' for the whole company
    reason     TEXT     NOT NULL,
    created_at DATETIME NOT NULL,
    expires_at DATETIME,
    UNIQUE (company_id, scope_type, scope_id)
);

INSERT INTO alert_silences_new (id, company_id, scope_type, scope_id, reason, created_at, expires_at)
SELECT s.id, p.company_id, s.scope_type, s.scope_id, s.reason, s.created_at, s.expires_at
FROM alert_silences s
JOIN checks k ON k.id = s.scope_id
JOIN probes p ON p.id = k.probe_id
WHERE s.scope_type = 'check';

DROP TABLE alert_silences;
ALTER TABLE alert_silences_new RENAME TO alert_silences;
CREATE INDEX idx_silences_expires ON alert_silences(expires_at);

-- A result that came during a maintenance window: no alert, and not counted
-- for uptime (the chart shades it).
ALTER TABLE check_results ADD COLUMN maintenance INTEGER NOT NULL DEFAULT 0;
