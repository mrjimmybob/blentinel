-- Incidents, acknowledgement and their timeline (ROADMAP M3-05, proposal
-- 0005 §2.3, D-67).

-- One outage of a check (check_id set) or of a probe that stopped reporting
-- (check_id NULL), from its first alert to its recovery. The names are as
-- they were when it opened: the history reads as it happened.
CREATE TABLE incidents (
    id              INTEGER  PRIMARY KEY AUTOINCREMENT,
    company_id      TEXT     NOT NULL,
    company_name    TEXT     NOT NULL,
    site_id         TEXT     NOT NULL,
    site_name       TEXT     NOT NULL,
    probe_id        TEXT     NOT NULL,
    probe_name      TEXT     NOT NULL,
    check_id        TEXT,                  -- NULL: the probe stopped reporting
    check_name      TEXT,
    severity        TEXT     NOT NULL,     -- the worst it got: Down | Warning
    status          TEXT     NOT NULL,     -- now: Down | Warning
    opened_at       DATETIME NOT NULL,
    closed_at       DATETIME,              -- NULL: open
    acknowledged_by TEXT,
    acknowledged_at DATETIME
);

-- At most one open incident per check, and per probe.
CREATE UNIQUE INDEX idx_incidents_open
    ON incidents(probe_id, IFNULL(check_id, '')) WHERE closed_at IS NULL;
CREATE INDEX idx_incidents_company ON incidents(company_id, opened_at);

-- What happened, in order: opened, changed, acknowledged, note, closed.
CREATE TABLE incident_events (
    id          INTEGER  PRIMARY KEY AUTOINCREMENT,
    incident_id INTEGER  NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    at          DATETIME NOT NULL,
    kind        TEXT     NOT NULL,
    username    TEXT,                      -- NULL: the hub
    text        TEXT     NOT NULL
);

CREATE INDEX idx_incident_events_incident ON incident_events(incident_id, id);
