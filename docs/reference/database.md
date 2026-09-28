# Database

The hub stores everything in one SQLite file (`db_path`, WAL mode). **The schema is defined by the migrations in [`hub/migrations/sqlite/`](../../hub/migrations/sqlite/)**, which are the source of truth and are applied automatically at startup. This page explains what the tables are for; the migration files have the exact columns.

Only `hub/src/storage/sqlite/` reads or writes these tables (ENGINEERING §2.2).

| Table | One row per | Notes |
|---|---|---|
| `reports` | accepted probe report | `probe_id`, `company_id`, `hostname`, `site`, `timestamp` (probe clock), `received_at`. Index `(probe_id, timestamp)` serves the replay check. |
| `resource_statuses` | resource in a report | status, message, latency, metric. Deleted with their report (`ON DELETE CASCADE`). |
| `probe_heartbeats` | known probe | `status` (`active`/`expired`), `last_seen_at` (hub clock), `first_seen_at` |
| `alert_states` | alert key | Last status seen and when an alert was last sent. Keys: `company:probe:name:target` (Up/Down), the same + `-threshold` (metric thresholds), `probe_expiry:company:probe` (probe offline). |
| `alert_silences` | silence | `scope_type` (currently always `resource`), `scope_id` (the resource key), reason, optional `expires_at` |
| `archives` | archive file created | filename, cutoff date, size |
| `users` | user account | `username` (unique, case-insensitive), `role`, argon2id `password_hash`, `is_active`, `must_change_password`, `all_companies` |
| `user_companies` | user × assigned company | Used from M1-04 (company scoping) |
| `sessions` | login session | `id_hash` = SHA-256 of the cookie value (the value itself is never stored), `last_seen_at` (idle expiry), `expires_at` (absolute). Archives contain no users or sessions. |
| `_sqlx_migrations` | applied migration | Managed by sqlx |

## Changing the schema

Add a new migration file with the next number; never edit a committed one. See [CONTRIBUTING.md](../../CONTRIBUTING.md#database-changes). While Blentinel is pre-alpha, old data isn't migrated across breaking changes (DECISIONS D-30).

## The probe's buffer

Each probe keeps undelivered reports in its own SQLite file, `blentinel_blackbox.db`:

| Table | Contents |
|---|---|
| `pending_reports` | sealed reports waiting for delivery, with the time they were queued |
| `meta` | the probe and hub keys the reports were made with. If either key changes, the buffer is discarded, since its reports could no longer be opened or verified. |
