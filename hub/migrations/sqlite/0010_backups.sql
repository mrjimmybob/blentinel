-- Scheduled backups (ROADMAP M2-12, proposal 0004 §4.5, D-60).
-- The backup files are on disk (`[backups] path`); the database holds when
-- they run and how the last scheduled one went.

-- The schedule set on the admin page; no row: the default (daily, 03:00 UTC, keep 7).
CREATE TABLE backup_schedule (
    id         INTEGER  PRIMARY KEY CHECK (id = 1),
    enabled    INTEGER  NOT NULL,
    hour_utc   INTEGER  NOT NULL,
    keep       INTEGER  NOT NULL,
    updated_by TEXT,                       -- username
    updated_at DATETIME NOT NULL
);

-- The last scheduled backup: the next is due at the first scheduled hour after it.
CREATE TABLE backup_last_run (
    id     INTEGER  PRIMARY KEY CHECK (id = 1),
    ran_at DATETIME NOT NULL,
    error  TEXT                            -- NULL: it worked
);
