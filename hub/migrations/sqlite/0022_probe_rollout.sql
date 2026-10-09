-- Managed probe rollout (ROADMAP M4-01, proposal 0012 §7, D-74): the
-- approved release goes to the canary probes first, then to every probe;
-- each company follows it, takes none, or is pinned to one release, within
-- an optional update window.

-- Replaces 0008's approval: the rollout holds the version and its stage.
DROP TABLE probe_update_approval;

-- No row (the default): no release approved.
CREATE TABLE probe_rollout (
    id                INTEGER  PRIMARY KEY CHECK (id = 1),
    version           TEXT     NOT NULL,
    approved_by       TEXT,                 -- username
    approved_at       DATETIME NOT NULL,
    stage             TEXT     NOT NULL CHECK (stage IN ('canary', 'all')),
    soak_minutes      INTEGER  NOT NULL,
    canaries_ready_at DATETIME,             -- since every canary runs it
    promoted_at       DATETIME,             -- sent to every probe
    promoted_by       TEXT,                 -- NULL with promoted_at: the hub
    paused_at         DATETIME,
    paused_by         TEXT,                 -- NULL with paused_at: the hub
    pause_reason      TEXT
);

-- Canaries get a release before the others.
ALTER TABLE probes ADD COLUMN update_canary INTEGER NOT NULL DEFAULT 0;

-- No row: the company follows the rollout, at any time.
CREATE TABLE company_update_policy (
    company_id   TEXT     PRIMARY KEY REFERENCES companies(id) ON DELETE CASCADE,
    mode         TEXT     NOT NULL CHECK (mode IN ('follow', 'off', 'pin')),
    version      TEXT,                      -- with 'pin'
    window_start TEXT,                      -- HH:MM, local to each probe's site
    window_end   TEXT,
    updated_by   TEXT,
    updated_at   DATETIME NOT NULL
);
