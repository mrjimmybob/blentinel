-- Host inventory and check templates (ROADMAP M5-12, proposal 0006 §4-5, D-85).

-- What each probe last said its machine is made of (protocol v9), as the
-- JSON of common::inventory::Inventory, and when. Shown, and read to suggest
-- checks; never queried, so one column.
ALTER TABLE probes ADD COLUMN inventory TEXT;
ALTER TABLE probes ADD COLUMN inventory_at DATETIME;

-- Templates people saved: a named set of checks to add to a probe. Hub-wide
-- (no company). `checks` is JSON: [{"name": …, "spec": {…}}]. The built-in
-- templates are code, not rows.
CREATE TABLE check_templates (
    id          TEXT     PRIMARY KEY,
    name        TEXT     NOT NULL COLLATE NOCASE UNIQUE,
    description TEXT     NOT NULL DEFAULT '',
    checks      TEXT     NOT NULL,
    created_at  DATETIME NOT NULL,
    created_by  TEXT
);
