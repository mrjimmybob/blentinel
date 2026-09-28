-- Per-user company scope (ROADMAP M1-04, D-45): all companies, only the
-- listed ones, or all except the listed ones. `user_companies` holds the list
-- for both `only` and `except`. Replaces `users.all_companies`, which is no
-- longer read (SQLite keeps the column).

ALTER TABLE users ADD COLUMN company_scope TEXT NOT NULL DEFAULT 'all'; -- all | only | except

UPDATE users SET company_scope = CASE WHEN all_companies = 1 THEN 'all' ELSE 'only' END;
