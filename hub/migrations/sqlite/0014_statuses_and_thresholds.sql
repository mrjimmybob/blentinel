-- Four statuses and the threshold hierarchy (ROADMAP M3-01, proposal 0005
-- §2.1, D-16, D-64). The hub judges each result against the thresholds that
-- apply (check, site, company, the hub's default) when it arrives, and keeps
-- that verdict with it.

-- The hub's default thresholds (at most one row; none: the built-in ones).
CREATE TABLE threshold_defaults (
    id         INTEGER  PRIMARY KEY CHECK (id = 1),
    thresholds TEXT     NOT NULL,     -- model::Thresholds as JSON
    updated_by TEXT,
    updated_at DATETIME NOT NULL
);

-- A company's and a site's thresholds (NULL: the level above decides).
ALTER TABLE companies ADD COLUMN thresholds TEXT;
ALTER TABLE sites ADD COLUMN thresholds TEXT;

-- A check's own limits for its metric (NULL: the levels above decide).
ALTER TABLE checks ADD COLUMN limits TEXT;

-- check_results.status is now the hub's verdict: 'Up', 'Warning' or 'Down';
-- why, when a threshold decided it.
ALTER TABLE check_results ADD COLUMN reason TEXT;

-- A check has one alert state now (Up, Warning, Down); the separate
-- threshold state goes (pre-alpha, D-30).
DELETE FROM alert_states WHERE resource_key LIKE '%-threshold';
