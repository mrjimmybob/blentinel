-- Secrets on a check (ROADMAP M5-08, D-86): what a check needs that nobody
-- may read back (an SNMP community, the v3 passwords). JSON: a secret's name
-- to its value sealed with the hub's data key (hub_data.key, D-66), so the
-- names can be listed without opening anything. NULL: none.
ALTER TABLE checks ADD COLUMN secrets TEXT;
