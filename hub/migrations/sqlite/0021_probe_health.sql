-- Probe health telemetry (ROADMAP M4-02, D-73): what each probe last said
-- about itself (protocol v8), as the JSON of common::models::ProbeHealth:
-- when it started, the reports it held back, its clock against the hub's,
-- its last problem. Only shown, never queried, so one column.
ALTER TABLE probes ADD COLUMN health TEXT;
