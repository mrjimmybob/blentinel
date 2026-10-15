-- Rotating the hub's TLS certificate and key over the protocol (ROADMAP
-- M2-13, proposal 0002 §4.5, D-61). The next and previous keys and
-- certificates are files beside the current ones; the database holds the
-- rotation's progress and which pins each probe says it holds.

CREATE TABLE hub_rotations (
    id              INTEGER  PRIMARY KEY AUTOINCREMENT,
    rotates_tls     INTEGER  NOT NULL,      -- the TLS certificate (and its key)
    rotates_key     INTEGER  NOT NULL,      -- the hub's X25519 key (and the signing key derived from it)
    window_days     INTEGER  NOT NULL,      -- after which a rotation still waiting is flagged
    started_by      TEXT,                   -- username
    started_at      DATETIME NOT NULL,
    deadline_at     DATETIME NOT NULL,
    state           TEXT     NOT NULL,      -- announcing | switched | cancelled
    ended_by        TEXT,                   -- username; NULL with ended_at: the hub (all probes ready)
    ended_at        DATETIME,
    deadline_warned INTEGER  NOT NULL DEFAULT 0
);

CREATE INDEX idx_hub_rotations_state ON hub_rotations(state);

-- What each probe last said it pins (protocol v7): digests of its current
-- pins and of the next ones the hub announced (common::pins::HubPins::digest).
ALTER TABLE probes ADD COLUMN pins_current TEXT;
ALTER TABLE probes ADD COLUMN pins_next TEXT;
