-- Hub upgrades (ROADMAP M4-04, proposal 0012 §8, D-75): the admin page's
-- opt-in check for a newer hub release, and what it last found. Off by
-- default (no row): the hub contacts no outside server unless asked to.
CREATE TABLE hub_upgrade_check (
    id             INTEGER  PRIMARY KEY CHECK (id = 1),
    enabled        INTEGER  NOT NULL,
    updated_by     TEXT,
    updated_at     DATETIME NOT NULL,
    checked_at     DATETIME,            -- the last check, if any
    latest_version TEXT,                -- the release it found
    error          TEXT                 -- why it failed, if it did
);
