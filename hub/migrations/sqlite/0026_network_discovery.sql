-- Network discovery (ROADMAP M5-18, proposal 0006 §4, D-101).

-- Whether the company's probes may look at their networks: off until
-- someone allows it (some networks' owners forbid scanning).
ALTER TABLE companies ADD COLUMN discovery_allowed INTEGER NOT NULL DEFAULT 0;

-- What each probe last found on its network (protocol v10), as the JSON of
-- common::discovery::Discovery, and when. Shown, and read to suggest
-- checks; never queried, so one column.
ALTER TABLE probes ADD COLUMN discovery TEXT;
ALTER TABLE probes ADD COLUMN discovery_at DATETIME;
