# Database

The hub stores everything in one SQLite file (`db_path`, WAL mode). **The schema is defined by the migrations in [`hub/migrations/sqlite/`](../../hub/migrations/sqlite/)**, which are the source of truth and are applied automatically at startup. This page explains what the tables are for; the migration files have the exact columns.

Only `hub/src/storage/sqlite/` reads or writes these tables (ENGINEERING §2.2).

| Table | One row per | Notes |
|---|---|---|
| `reports` | accepted probe report | `probe_id`, `company_id`, `hostname`, `site`, `timestamp` (probe clock), `received_at`. Index `(probe_id, timestamp)` serves the replay check. |
| `check_results` | check result in a report | `check_id`, status, message, latency. Deleted with their report (`ON DELETE CASCADE`). No foreign key to `checks`: history outlives a deleted check. |
| `check_metrics` | metric of a result | `key`, `value`, `unit`, `position` (the order reported; the first is the headline). Deleted with their result. |
| `checks` | check | `id` = the `CheckId` (random, hex), `probe_id` (deleted with its probe), `name`, `kind`, `spec` (the `CheckSpec` as JSON, `kind` included), `enabled`, `fail_after` + `recover_after` (results in a row that alert / recover; `NULL`: 2; migration `0013`, D-63), `depends_on` (the check of the same probe it is behind, `NULL` when that one is deleted; migration `0018`, D-68), `created_at`, `updated_at`, `updated_by` (D-49) |
| `probe_heartbeats` | known probe | `status` (`active`/`expired`), `last_seen_at` (hub clock), `first_seen_at` |
| `alert_states` | alert key (column `resource_key`) | The confirmed status (`last_status`: what people were last told), `streak` (results in a row since then that say otherwise, D-63) and when an alert was last sent. Keys: the check ID (its status: `Up`, `Warning`, `Down`; D-64), `probe_expiry:company:probe` (probe offline). |
| `alert_silences` | silence (made again in migration `0015`, D-65) | `company_id`, `scope_type` (`company`, `site`, `probe`, `check`), `scope_id` (`''` for the company; else the site, probe or check ID), `reason`, `created_at`, optional `expires_at`; one per `(company_id, scope_type, scope_id)` |
| `archives` | archive file created | filename, cutoff date, size |
| `users` | user account | `username` (unique, case-insensitive), `role`, argon2id `password_hash`, `is_active`, `must_change_password`, `company_scope` (`all` / `only` / `except`); `all_companies` is unused since migration 0004 |
| `user_companies` | user × listed company | The companies of an `only` or `except` scope |
| `companies` | company | `id` (a slug, immutable), display `name`, contacts, notes; `reviewed_at` empty = never reviewed ("New"; companies created in the UI start reviewed); `archived_at` hides it from the dashboard |
| `sites` | site of a company | key `(company_id, id)`; display `name`, `address`, notes |
| `probes` | enrolled probe | `id` = its probe ID (Ed25519 public key, hex), `name`, the `company_id` and `site_id` the hub assigned (reports are stored under these), `state` (`active`/`revoked`), `hostname`, `os`, `arch`, `probe_version` (from its latest report; at enrollment until then), `enrolled_at`, `revoked_at`, `token_id` (the code used); `interval_secs` (sent with its checks), `config_revision` (what it last said it runs), `config_rejected_revision` + `config_error` (the last config it refused), `update_failed_version` + `update_failed_error` (the last update it refused or rolled back, D-55), `pins_current` + `pins_next` (digests of the hub pins it said it holds, protocol v7, D-61). Enrolling again keeps these and its checks. |
| `enrollment_tokens` | enrollment code | `id` (the token ID in the code), `secret_hash` = SHA-256 of its secret (the code itself is never stored), `company_id`, `site_id`, `probe_name`, `created_by`, `expires_at`, `max_uses` (0: no limit), `uses`, `revoked_at`. Not copied into archives. |
| `audit_log` | recorded action | `at`, `user_id` + `username` as they were (`NULL` = the hub or the console), `actor_ip`, `action`, `target_type`/`target_id`, `company_id`, `details` (JSON). Append-only; not copied into archives. |
| `sessions` | login session | `id_hash` = SHA-256 of the cookie value (the value itself is never stored), `last_seen_at` (idle expiry), `expires_at` (absolute). Archives contain no users or sessions. |
| `probe_update_approval` | approved probe release (at most one row, `id = 1`) | `version` offered to probes, `approved_by`, `approved_at`. No row: updates are off (the default). The releases themselves are files in `<data folder>/releases/` (D-55). |
| `probe_commands` | command for a probe (migration `0009`, D-56) | `probe_id` (deleted with the probe), `kind` and `spec` (JSON, `common::commands::CommandSpec`), `created_by`, `created_at`, `expires_at`, `state` (`queued`, `delivered`, `succeeded`, `failed`, `cancelled`; "expired" is worked out on reading), `delivered_at` (first sent), `completed_at`, `result` (the probe's message or the hub's reason). Not copied into archives. |
| `backup_schedule` | the backup schedule (at most one row, `id = 1`; migration `0010`, D-60) | `enabled`, `hour_utc`, `keep`, `updated_by`, `updated_at`. No row: the default (daily, 03:00 UTC, keep 7). The backups themselves are files in the backups folder. |
| `backup_last_run` | the last scheduled backup (at most one row) | `ran_at`, `error` (`NULL`: it worked). The next is due at the first scheduled hour after `ran_at`. |
| `hub_rotations` | rotation of the hub's certificate and/or key (migration `0011`, D-61) | `rotates_tls`, `rotates_key`, `window_days`, `started_by`, `started_at`, `deadline_at`, `state` (`announcing`, `switched`, `cancelled`; at most one announcing), `ended_by` (`NULL` with `ended_at`: the hub switched by itself), `ended_at`, `deadline_warned`. The keys and certificates are files beside the current ones (`*.next.*`, `hub_identity.prev.key`). |
| `maintenance_windows` | a maintenance window (migration `0015`, D-65) | `company_id` (deleted with it), `scope_kind` (`company`, `site`, `probe`, `check`), `scope_id` (`NULL` for the company), `reason`, `timezone` (IANA), `schedule` (JSON: `model::Schedule`), `created_by`, `created_at`. `sites.timezone` is a site's zone (default `UTC`); `check_results.maintenance` is 1 for a result that came during a window (left out of uptime). |
| `incidents` | an outage of a check (`check_id`) or of a probe that stopped reporting (`check_id` `NULL`) (migration `0017`, D-67) | `company_id`, `company_name`, `site_id`, `site_name`, `probe_id`, `probe_name`, `check_id`, `check_name` (names as they were when it opened), `severity` (the worst), `status` (now), `opened_at`, `closed_at` (`NULL`: open; at most one open per check and per probe, a partial unique index), `acknowledged_by`, `acknowledged_at`, and for reminders (migration `0019`, D-69) `announced` (0: it opened silenced, nobody was told), `reminders`, `last_reminded_at`, `escalated_at`. Deleted with the probe's or company's data. |
| `incident_events` | an incident's timeline | `incident_id` (deleted with it), `at`, `kind` (`opened`, `changed`, `acknowledged`, `reminded`, `escalated`, `note`, `closed`), `username` (`NULL`: the hub), `text` |
| `alert_settings` | the mail server and the hub's address for links (at most one row, `id = 1`; migration `0016`, D-66) | `hub_url`, `smtp_server`, `smtp_port`, `smtp_security` (`starttls`, `tls`, `none`), `smtp_username`, `smtp_from`, `smtp_password_sealed` (sealed with `hub_data.key`), `updated_by`, `updated_at`. No row: nothing set. |
| `alert_channels` | where alerts can go (D-66) | `name`, `kind` (`email`, `webhook`, `teams`, `slack`, `discord`, `telegram`, `ntfy`), `enabled`, `target` (addresses, a URL, a chat ID or an ntfy topic URL), `secret_sealed` (a webhook URL, signing key, bot token or ntfy access token, sealed with `hub_data.key`), `created_at`, `last_delivery_at`, `last_error` (`NULL`: the last one worked) |
| `digest_settings` | the daily digest (at most one row, `id = 1`; migration `0020`, D-71) | `hour` (0–23, local), `timezone` (IANA), `channels` (JSON array of e-mail channel IDs), `last_sent` (the local date it last went, `YYYY-MM-DD`), `updated_by`, `updated_at` |
| `alert_routes` | the routing rules, in order (D-66) | `position`, `rule` (JSON: `model::RouteRule`: `name`, `companies`, `sites`, `kinds`, `check_kinds`, `channels` (channel IDs), `stop`, `remind_every_minutes`, `escalate_after_minutes`, `escalate_to` (D-69)) |
| `threshold_defaults` | the hub's default thresholds (at most one row; migration `0014`, D-64) | `thresholds` (JSON: `cpu`, `memory`, `disk`, `latency`, each `{warning, down}`, a number or `"off"`), `updated_by`, `updated_at`. No row: the built-in defaults. `companies.thresholds` and `sites.thresholds` hold the same JSON (`NULL`: nothing set), `checks.limits` one metric's `{warning, down}`. `check_results.status` is the hub's verdict (`Up`, `Warning`, `Down`) and `check_results.reason` why, when a threshold decided it. |
| `acme_settings` | the HTTPS certificate from an ACME authority (at most one row, `id = 1`; migration `0012`, D-62) | `enabled`, `domains` (JSON array), `email`, `authority` (`lets_encrypt`, `lets_encrypt_staging`, `custom`), `directory_url` and `ca_pem` (custom only), `challenge` (`tls_alpn`, `http`), `terms_agreed`, `updated_by`, `updated_at`. No row: off. The certificate and the ACME account's key (`hub_acme_account.json`) are files. |
| `acme_status` | the last request for a certificate (at most one row) | `last_attempt_at`, `last_error` (`NULL`: it worked), `failures` (in a row; retries wait 1 h, doubling up to 24 h), `last_issued_at` |
| `_sqlx_migrations` | applied migration | Managed by sqlx |

## Changing the schema

Add a new migration file with the next number; never edit a committed one. A hub that finds an older schema backs up first and puts the database back if a migration fails (D-60), so a migration that fails half-way leaves nothing behind; still, make each one correct. See [CONTRIBUTING.md](../../CONTRIBUTING.md#database-changes). While Blentinel is pre-alpha, old data isn't migrated across breaking changes (DECISIONS D-30).

## The probe's buffer

Each probe keeps undelivered reports in its own SQLite file, `blentinel_blackbox.db`:

| Table | Contents |
|---|---|
| `pending_reports` | sealed reports waiting for delivery, with the time they were queued |
| `meta` | the probe and hub keys the reports were made with. If either key changes, the buffer is discarded, since its reports could no longer be opened or verified. |
