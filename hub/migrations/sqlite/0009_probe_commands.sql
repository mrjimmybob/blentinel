-- The command channel (ROADMAP M2-06, proposal 0003 §4.3, D-56).
-- A command waits here until it rides in the answer to the probe's next
-- report; its result comes back in the report after that.

CREATE TABLE probe_commands (
    id           TEXT     PRIMARY KEY,       -- 16 random bytes, hex
    probe_id     TEXT     NOT NULL REFERENCES probes(id) ON DELETE CASCADE,
    kind         TEXT     NOT NULL,          -- run_checks
    spec         TEXT     NOT NULL,          -- common::commands::CommandSpec as JSON (kind included)
    created_by   TEXT,                       -- username
    created_at   DATETIME NOT NULL,
    expires_at   DATETIME NOT NULL,          -- not sent after this; shown as expired if not done
    state        TEXT     NOT NULL,          -- queued | delivered | succeeded | failed | cancelled
    delivered_at DATETIME,                   -- first sent to the probe
    completed_at DATETIME,
    result       TEXT                        -- the probe's message, or the hub's reason
);

CREATE INDEX idx_probe_commands_probe ON probe_commands(probe_id, created_at);
