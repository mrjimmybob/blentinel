-- Check dependencies (ROADMAP M3-07, proposal 0005 §2.5, D-68): a check of
-- the same probe this one is behind, usually the site's gateway ping. While
-- it is down, this check's results are Unknown (unreachable). Deleting the
-- parent clears the link.
ALTER TABLE checks ADD COLUMN depends_on TEXT REFERENCES checks(id) ON DELETE SET NULL;
