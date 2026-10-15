-- Reminders and escalation of unacknowledged incidents (ROADMAP M3-06,
-- proposal 0005 §2.3, D-69). Kept with the incident, so a restart neither
-- loses nor repeats them.
ALTER TABLE incidents ADD COLUMN announced INTEGER NOT NULL DEFAULT 1; -- 0: opened silenced
ALTER TABLE incidents ADD COLUMN reminders INTEGER NOT NULL DEFAULT 0;
ALTER TABLE incidents ADD COLUMN last_reminded_at DATETIME;
ALTER TABLE incidents ADD COLUMN escalated_at DATETIME;
