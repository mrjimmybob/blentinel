-- Flap suppression (ROADMAP M3-02, proposal 0005 §2.2, D-63): a check's alert
-- state changes only after several results in a row say so.

-- Results in a row that disagree with last_status (the confirmed status).
ALTER TABLE alert_states ADD COLUMN streak INTEGER NOT NULL DEFAULT 0;

-- Per check: failed / good results in a row that change it; NULL: the default (2).
ALTER TABLE checks ADD COLUMN fail_after INTEGER;
ALTER TABLE checks ADD COLUMN recover_after INTEGER;
