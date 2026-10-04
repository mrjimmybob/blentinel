-- Probe self-update, phase 1 (ROADMAP M2-15, proposal 0012, D-41, D-55).
-- The release files are on disk (<state_dir>/releases/<version>/); the
-- database holds which one the admin approved, and what each probe says.

-- The last update a probe refused or rolled back (protocol v5), cleared when
-- it reports without one. `probe_version` (0006) now follows every report.
ALTER TABLE probes ADD COLUMN update_failed_version TEXT;
ALTER TABLE probes ADD COLUMN update_failed_error TEXT;

-- The release offered to probes; no row (the default): none, no updates.
CREATE TABLE probe_update_approval (
    id          INTEGER  PRIMARY KEY CHECK (id = 1),
    version     TEXT     NOT NULL,
    approved_by TEXT,                    -- username
    approved_at DATETIME NOT NULL
);
