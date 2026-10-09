# HTTP API reference

The hub's JSON API, used by the web UI and available to scripts. The probe endpoints are in [protocol.md](protocol.md). The route table is `hub/src/routes.rs`; the JSON shapes are defined once in `hub/src/model.rs`.

## Authentication and access

The API is served over **HTTPS** (port 3443 by default); on the plain-HTTP port only the probe endpoints (and an ACME authority's HTTP-01 requests) answer, anything else is redirected there ([install/https.md](../install/https.md)).

**Every POST needs the header `X-Blentinel: 1`** (CSRF protection), except the probe endpoints; without it: **403** `csrf`. For example:

```sh
curl -k -c jar -H 'X-Blentinel: 1' -H 'Content-Type: application/json' \
     -d '{"username":"alice","password":"…"}' https://hub:3443/api/auth/login
```

Log in to get a `blentinel_session` cookie (`HttpOnly; SameSite=Strict`, `Secure` over HTTPS; ends after 12 idle hours or 7 days). Every other endpoint needs it and answers **401** (`unauthorized`) without a valid one, except the probe endpoints, login and setup.

Each endpoint below names the **permission** it needs. Roles have these permissions ([guide/users.md](../guide/users.md)):

| Permission | Owner | Admin | Technician | Viewer |
|---|---|---|---|---|
| `ViewData`: dashboards, companies, uptime, devices, archives, silence list | ✓ | ✓ | ✓ | ✓ |
| `Silence`: create and remove silences | ✓ | ✓ | ✓ | |
| `ViewAdmin`: admin listings, storage size, archive list | ✓ | ✓ | | |
| `ManageData`: add companies, revoke and remove probes, delete data, create archives | ✓ | ✓ | | |
| `ManageUsers`: users | ✓ | ✓ | | |
| `ManageCompanies`: edit companies, add sites and probes (within the scope) | ✓ | ✓ | ✓ | |

Clients have `ViewData` only. Without the permission: **403** `forbidden`.

**Company scope** (D-45): every answer is limited to the user's companies. Lists (`/api/dashboard/companies`, `/api/companies`, `/api/silences`, archive company lists) leave the others out; a single company, probe, silence or archive item outside the scope answers **404** `not_found`, exactly as if it didn't exist. Clients get company and site answers without `notes` and `contact_*`. A user with a temporary password gets **403** `password_change_required` everywhere except `/api/auth/me`, `/api/me/password` and `/api/auth/logout`. The table is `hub/src/routes.rs`; `hub/tests/access.rs` checks every route against it.

| Method | Path | Access | Body | Result |
|---|---|---|---|---|
| `POST` | `/api/auth/login` | anyone | `{"username", "password"}` | The user (see below) + `Set-Cookie`; 401 `invalid_credentials`; 429 `too_many_attempts` (with `Retry-After`) after repeated failures ([guide/users.md](../guide/users.md#your-account)) |
| `POST` | `/api/auth/logout` | logged in | none | Ends this session, clears the cookie |
| `GET` | `/api/auth/me` | logged in | none | The logged-in user |
| `GET` | `/api/setup` | anyone | none | `{"needed": true}` while the hub has no users |
| `POST` | `/api/setup` | anyone | `{"code", "username", "display_name", "password"}` | Creates the first Owner and logs in (like login). 403 `wrong_setup_code`, 409 `already_set_up` |
| `POST` | `/api/me/password` | logged in | `{"current_password", "new_password"}` | Ends your other sessions. 400 `wrong_password`, `weak_password` |
| `POST` | `/api/me/logout-others` | `ViewData` | none | Ends your other sessions |

A user:

```json
{
  "id": "5f2c…", "username": "alice", "display_name": "Alice", "email": null,
  "role": "technician", "scope": { "mode": "only", "companies": ["acme", "beta"] },
  "is_active": true, "must_change_password": false,
  "created_at": "2026-09-28T10:00:00Z", "last_login_at": "2026-09-28T10:05:00Z"
}
```

## Users

All need `ManageUsers`. Only an Owner may create, edit, reset or log out Owners (403 `forbidden`).

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/users` | none | All users, by username |
| `POST` | `/api/users` | `{"username", "display_name", "role", "scope"?, "email"?}` | `{"user", "temporary_password"}`: shown once; must be changed at first login. 409 `username_taken`, 400 `invalid_username` |
| `POST` | `/api/users/{id}` | `{"display_name", "role", "scope"?, "is_active", "email"?}` | The updated user. Deactivating ends their sessions. 409 `last_owner`; 403 when deactivating yourself |
| `POST` | `/api/users/{id}/reset-password` | none | `{"user", "temporary_password"}`; ends their sessions |
| `POST` | `/api/users/{id}/logout` | none | Ends all their sessions |

`role` is `owner`, `admin`, `technician`, `viewer` or `client`. `scope` is `{"mode": "all"}` (the default), `{"mode": "only", "companies": [...]}` or `{"mode": "except", "companies": [...]}`, with company IDs. Owners and Admins always get `all`; `except` with no companies is `all`. Refused with 400 `invalid_scope`: `only` with no companies, and a Client with anything but `only`. A company that isn't a record: 400 `unknown_company`.

## Errors

Every error response, here and on the probe endpoint, has the same JSON body:

```json
{ "error": { "code": "not_found", "message": "archive 42 not found or unreadable" } }
```

`code` is stable, so scripts should match on it. `message` is for people and may change.

| Status | `code` | When |
|---|---|---|
| 401 | `unauthorized` | No valid session |
| 401 | `invalid_credentials` | Wrong username or password (the same answer for both) |
| 403 | `forbidden`, `password_change_required`, `wrong_setup_code`, `csrf` | See [Authentication and access](#authentication-and-access) |
| 429 | `too_many_attempts` | Too many failed logins or setup codes; `Retry-After` says when to try again |
| 400 | `weak_password`, `wrong_password`, `invalid_username`, `invalid_scope`, `unknown_company` | See [Users](#users) |
| 400 | `invalid_query` | See [Audit log](#audit-log) |
| 409 | `username_taken`, `last_owner`, `already_set_up` | See [Users](#users) |
| 400 / 415 / 422 | `invalid_body` | The request body isn't the expected JSON (`message` says what's wrong) |
| 404 | `not_found` | The archive doesn't exist or can't be opened |
| 400 / 409 | `untrusted_release`, `invalid_release`, `unsupported_protocol_version`, `release_exists`, `release_approved` | See [Probe releases and updates](#probe-releases-and-updates) |
| 500 | `internal` | A failure on the hub, e.g. the database. The details are in the hub log, never in the response. |

The probe endpoint's codes are in [protocol.md](protocol.md#responses-to-post-apireport). The type is `common::protocol::ErrorBody`, shared by the hub, the web UI and the probe.

## Companies and sites

Created and edited in the UI ([guide/companies-sites.md](../guide/companies-sites.md)). Reading needs `ViewData`, changing needs `ManageCompanies`.

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/companies` | none | All companies (archived ones too), by name: `id`, `name`, `notes`, `contact_name`, `contact_email`, `contact_phone`, `is_new`, `archived`, `created_at`, `site_count`, `probe_count` |
| `POST` | `/api/companies` (`ManageData`) | `{"id", "name"}` | The new company with its sites (none yet). 400 `invalid` for a bad ID (lowercase letters, digits, `-`) or an empty name; 409 `exists` |
| `GET` | `/api/companies/{id}` | none | `{"company": {…as above}, "sites": [{"id", "name", "address", "notes", "is_new"}]}` |
| `POST` | `/api/companies/{id}` | `{"name", "notes"?, "contact_name"?, "contact_email"?, "contact_phone"?}` | The updated company with its sites. 400 `invalid_name` if the name is empty |
| `POST` | `/api/companies/{id}/archive` (`ManageData`) | `{"archived": true}` | Hides it from the dashboard (reports and alerts continue); `false` shows it again |
| `POST` | `/api/companies/{id}/sites` | `{"id", "name"}` | The company with its sites. 400 `invalid`, 409 `exists` as above |
| `POST` | `/api/companies/{id}/sites/{site}` | `{"name", "address"?, "notes"?}` | The company with its sites |

IDs can't change: reports are stored under them. Unknown companies or sites (or ones outside the scope): 404.

## Probes and enrollment codes

D-47; how it works: [install/probe.md](../install/probe.md), [security.md](../security.md#enrollment). Everything is limited to the user's company scope.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/probes` | `ViewData` | none | Enrolled probes: `id`, `name`, `company_id`, `company_name`, `site_id`, `site_name`, `state` (`active`/`revoked`), `hostname`, `os`, `arch`, `probe_version` (from its latest report), `enrolled_at`, `last_seen_at`, `interval_secs`, `update_failed` (`{"version", "error"}`: the last update it refused or rolled back, or `null`), `silent` (`true`: no report for the hub's probe timeout), `config_revision` and `config_rejected` (`{"revision", "error"}`) as the probe last said, `config_sync` (`in_sync`, `pending` or `rejected`, as in `GET /api/probes/{id}/checks`), `health` (what the probe last said about itself, protocol v8: `started_at`, `queued_reports`, `clock_offset_secs`, `last_error` `{"at", "message"}`; `null` before its first v8 report; [protocol.md](protocol.md#the-report)). The **Probes** page (the fleet view) is built from this. |
| `POST` | `/api/probes/{id}` | `ManageCompanies` | `{"name", "site_id", "interval_secs"}` | Renames it, moves it to another site of its company, sets how often it reports (5–3600 s; it gets the new interval with its checks). `{"success": true}` |
| `POST` | `/api/probes/{id}/revoke` | `ManageData` | none | Its reports are refused (`403 probe_revoked`) until it enrolls again. `{"success": true}` |
| `GET` | `/api/enrollment-codes?company=` | `ManageCompanies` | none | Codes that can still be used (not expired, used up or revoked): `id`, `company_id`, `site_id`, `probe_name`, `created_by`, `created_at`, `expires_at`, `max_uses` (`null`: no limit), `uses`. `company` is optional |
| `POST` | `/api/enrollment-codes` | `ManageCompanies` | `{"company_id", "site_id", "hub_url", "probe_name"?, "max_uses"? (1–1000; omitted or `null`: no limit), "valid_hours"? (1–720, default 24)}` | `{"code": "BLT1-…", "info": {…as above}, "hub_url"}`. The code is in this answer only |
| `POST` | `/api/enrollment-codes/{id}/revoke` | `ManageCompanies` | none | `{"success": true}` |
| `GET` | `/api/probe-downloads` | `ManageCompanies` | none | The probe binaries this hub offers ([install/hub.md](../install/hub.md#probe-binaries)): those of the approved release, else the newest one: `target`, `os`, `arch`, `size`, `sha256` (hex, from the signed manifest), `version`. The Add probe wizard builds its one-line commands from these |
| `GET` | `/download/probe/{target}` | none (also on the plain HTTP port) | none | That binary (`application/octet-stream`, header `x-checksum-sha256`). 404 if the hub has none for that target |

`hub_url` is the address probes will use (`http://` or `https://`). For an `https://` address the hub puts a pin of its own certificate's key in the code, if it serves TLS (D-61). Invalid values: 400 `invalid`.

Probes themselves call `POST /api/enroll` (no session, no CSRF header; the body is sealed and signed): see [protocol.md](protocol.md#enrollment).

## Checks

M2-04, D-49. What each probe checks is a record on the hub; the probe gets its enabled checks and its interval in the answer to its next report ([protocol.md](protocol.md#the-probes-checks-config)). The kinds and their settings: [checks.md](checks.md). Limited to the user's company scope: another company's probe or check answers 404.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/probes/{id}/checks` | `ViewData` | none | The probe's checks and whether it runs them (below) |
| `POST` | `/api/probes/{id}/checks` | `ManageCompanies` | `{"name", "spec", "enabled"? (default true), "fail_after"?, "recover_after"? (default null: 2), "limits"?, "depends_on"? (a check ID of the same probe, D-68)}` | The new check. 400 for a `depends_on` on another probe |
| `POST` | `/api/probes/{id}/checks/copy` | `ManageCompanies` | `{"from_probe_id"}` | Adds copies (new IDs, same names, specs, enabled flags, alert settings and links between them) of every check of another probe in the user's companies. The new checks. 400 if it's the same probe or the copies would pass 500 |
| `POST` | `/api/checks/{id}` | `ManageCompanies` | as above | Replaces name, spec, enabled flag, alert settings, limits and `depends_on` (missing: the default, so send them all); the ID stays, so its history, alert state and silence stay too. The check. 400 for a `depends_on` that is itself or would make a loop |
| `POST` | `/api/checks/{id}/delete` | `ManageCompanies` | none | Deletes it and its silence (its results stay in the history). `{"success": true}` |

A check:

```json
{
  "id": "5f0c1e…a1", "probe_id": "a773d2…dfcd", "name": "Core switch",
  "spec": { "kind": "tcp", "host": "192.0.2.1", "port": 443 }, "target": "192.0.2.1:443",
  "enabled": true, "created_at": "2026-09-30T10:00:00Z", "updated_at": "2026-09-30T10:00:00Z",
  "updated_by": "alice"
}
```

`GET /api/probes/{id}/checks`:

```json
{
  "probe_id": "a773d2…dfcd", "interval_secs": 60,
  "config_revision": "3f1a9c0e5b7d2468", "applied_revision": "3f1a9c0e5b7d2468",
  "sync": "in_sync", "config_error": null,
  "checks": [ { …a check… } ]
}
```

`sync` is `in_sync` (the probe runs `config_revision`), `pending` (it gets it with the answer to its next report) or `rejected` (it refused it; `config_error` says why). A check also has `limits` (its own thresholds), `limits_in_force` (what applies, each limit with `value` and `from`: `check`, `site`, `company`, `global`, `built_in`; `null` for `uptime`), and `fail_after` and `recover_after` (`null`: the default, 2; [checks.md](checks.md)). An invalid name, spec or count (not 1–10): 400 `invalid` with the reason (e.g. `ping needs an IP address, e.g. 192.168.1.1`); an unknown `kind`: 422 `invalid_body`. At most 500 checks per probe. Changes are in the audit log (`check.create`, `check.update`, `check.delete`, `check.copy`).

## Commands

M2-06, D-56. A command for a probe waits on the hub until it rides in the answer to the probe's next report; its result comes back with a later report ([protocol.md](protocol.md#commands-commands)). Limited to the user's company scope: another company's probe or command answers 404.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/probes/{id}/commands` | `ViewData` | none | The probe's latest 20 commands, newest first |
| `POST` | `/api/probes/{id}/commands` | `ManageCompanies` | `{"kind": "run_checks"}` | The command. If the same command is already waiting (queued or delivered), that one: clicking twice runs it once. 400 `invalid` for an unknown kind or a revoked probe |
| `POST` | `/api/commands/{id}/cancel` | `ManageCompanies` | none | The command, cancelled. 400 `invalid` once it has been sent to the probe |

A command:

```json
{
  "id": "9b2e…c4", "probe_id": "a773d2…dfcd", "kind": "run_checks",
  "created_by": "alice", "created_at": "2026-10-04T10:00:00Z", "expires_at": "2026-10-04T10:15:00Z",
  "state": "succeeded", "delivered_at": "2026-10-04T10:00:41Z", "completed_at": "2026-10-04T10:00:42Z",
  "result": "ran 3 checks"
}
```

`state` is `queued` (waiting for the probe's next report), `delivered` (sent; the result comes with a later report), `succeeded`, `failed` (`result` says why), `cancelled`, or `expired` (not done within 3 report intervals, at least 15 minutes). Queuing and cancelling are in the audit log (`command.queue`, `command.cancel`).

## Dashboard

All need `ViewData`.

### `GET /api/dashboard/companies`

One entry per company with probes, sorted by `company_id`. Archived companies are left out. `name` is the company's display name:

```json
[{
  "company_id": "example-company",
  "name": "Example Company Ltd",
  "total_probes": 2, "active_probes": 1, "expired_probes": 1,
  "devices_up": 5, "devices_down": 1, "devices_warning": 2,
  "severity": "critical",
  "last_report": "2026-03-10T12:01:00Z"
}]
```

`devices_up`/`devices_down`/`devices_warning` count the statuses of each probe's latest report, including stale ones from expired probes. `severity` (`critical` / `degraded` / `warning` / `healthy`) is what to display; the rule is in [architecture.md](../architecture.md#severity).

### `GET /api/company/{company_id}/probes`

The company's probes, most recently seen first:

```json
[{
  "probe_id": "a773d2…", "probe_name": "head-office",
  "hostname": "office-pc-01", "site": "head-office",
  "status": "active", "severity": "healthy",
  "last_seen_at": "2026-03-10T12:00:30Z",
  "devices_up": 3, "devices_down": 0, "devices_warning": 1
}]
```

`status` is `active` or `expired`. `probe_name` is the name in the probe's record.

### `GET /api/company/{company_id}/uptime?range=24h|7d|30d|all`

Up/down counts per time bucket, oldest first. Buckets are UTC (`"YYYY-MM-DD HH:MM"`):

| `range` | Window | Bucket |
|---|---|---|
| `24h` (default; also any unknown value) | last 24 hours | 15 min |
| `7d` | last 7 days | 1 hour |
| `30d` | last 30 days | 4 hours |
| `all` | everything stored | 1 day |

```json
[{ "bucket": "2026-03-10 10:00", "up_count": 2, "down_count": 1 }]
```

`up_count` includes Warning results: they worked.

### `GET /api/probe/{probe_id}/devices`

The results of the probe's latest report, one per check (checks deleted since are left out), sorted by name:

```json
[{
  "check_id": "9b77…3e", "name": "CPU", "kind": "cpu", "target": "localhost",
  "status": "Warning", "reason": "cpu usage 96.2% is over the warning limit (95%, from the built-in default)",
  "message": "CPU usage: 96.2%", "latency_ms": null,
  "metrics": [{ "key": "usage", "value": 96.2, "unit": "%" }]
}]
```

`status` is the hub's verdict (D-64): `Up`, `Warning`, `Down`, or `Unknown` while the probe is expired. `reason` says which threshold decided it (`null` otherwise). The first metric is the headline value the UI shows.

## Audit log

Owner and Admin (`ViewAdmin`). What is recorded: [guide/audit.md](../guide/audit.md).

| Method | Path | Result |
|---|---|---|
| `GET` | `/api/audit` | Entries, newest first: `id`, `at`, `username` (`null` = the hub or the server console), `actor_ip`, `action`, `target_type`, `target_id`, `company_id`, `details` (JSON text) |
| `GET` | `/api/audit.csv` | The same as a CSV file (up to 1000 entries) |

Query parameters, all optional: `user` (username, any case), `action` (an action or a group: `user` matches `user.create`, `user.update`, …), `target` (a target or company ID), `from` / `to` (RFC 3339 times; `to` is exclusive), `before_id` (the next page: entries older than this ID), `limit` (default 200, at most 1000). A bad time: 400 `invalid_query`.

## Alert silences

Listing needs `ViewData`; creating and removing need `Silence`.

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/silences` | none | Silences that haven't expired: `id`, `company_id`, `scope`, `reason`, `created_at`, `expires_at` |
| `POST` | `/api/silence` | `{"company_id", "scope", "reason", "duration_hours"}` | `{"success": true, "silence_id": 1}` |
| `POST` | `/api/silence/delete` | `{"silence_id": 1}` | `{"success": true}` |

A `scope` is `{"kind": "company"}`, `{"kind": "site", "site_id"}`, `{"kind": "probe", "probe_id"}` or `{"kind": "check", "check_id"}` (D-65); a check's silence survives renaming or retargeting it. `duration_hours` is `null` for "until removed" (a check's also ends when it recovers). Silencing the same scope again replaces its silence. A company outside the user's: 404; a scope that isn't in the company: 400 `invalid_maintenance`. See [guide/alerting.md](../guide/alerting.md#silences).

## Maintenance windows

Planned work during which a scope sends no alerts and its results don't count for uptime (M3-04, D-65; how: [guide/alerting.md](../guide/alerting.md#maintenance-windows)). Audited as `maintenance.create` and `maintenance.delete`.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/companies/{id}/maintenance` | `ViewData` | none | `windows` (each: `window` with `id`, `company_id`, `scope`, `reason`, `timezone`, `schedule`, `created_by`, `created_at`; `scope_label`, `schedule_text`, `active`, `next`: the current or next opening in its zone, `null` once over), `silences` (the company's), `sites` (with `timezone`), `probes` (`probe_id`, `probe_name`, `site_id`, `checks` as `[id, name]`) |
| `POST` | `/api/companies/{id}/maintenance` | `Silence` | `{"scope", "reason", "timezone", "schedule"}` | The window. 400 `invalid_maintenance` (no reason, not a time zone, a bad schedule, a scope not in the company) |
| `POST` | `/api/maintenance/{id}/delete` | `Silence` | none | `{"success": true}`; 404 if none in the user's companies |

A `schedule` is `{"every": "once", "start": "2026-10-10T09:00", "end": "2026-10-10T17:00"}` (local times), `{"every": "weekly", "days": [2, 4], "start": "22:00", "minutes": 240}` (1 = Monday … 7 = Sunday), or `{"every": "monthly", "on": {"by": "weekday", "week": 2, "weekday": 2}, "start": "22:00", "minutes": 180}` (`week` 5 = the last; or `{"by": "date", "day": 1}`). At most 7 days long. `timezone` is an IANA name (`Europe/Madrid`, `UTC`).

Sites: `POST /api/companies/{id}/sites/{site}` takes `timezone` too (IANA; default `UTC`; 400 `invalid_timezone`). Results: `GET /api/probe/{id}/devices` has `in_maintenance`; uptime buckets have `maintenance_count` (left out of `up_count`/`down_count`).

## Incidents

Outages from their first alert to their recovery (M3-05, D-67; how: [guide/alerting.md](../guide/alerting.md#incidents)). An incident is `id`, `company_id`, `company_name`, `site_id`, `site_name`, `probe_id`, `probe_name`, `check_id` and `check_name` (`null`: the probe stopped reporting; names as they were when it opened), `severity` (the worst: `Down` or `Warning`), `status` (now, or when it closed), `opened_at`, `closed_at` (`null`: open), `duration` (`1 h 5 min`, so far while open), `acknowledged_by`, `acknowledged_at`, `reminders` (sent so far) and `escalated_at` (D-69). Audited as `incident.acknowledge` and `incident.note`.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/incidents?state=open` | `ViewData` | none | The user's companies' incidents, newest first, at most 500: `state` `open` (default), `closed` or `all`. 400 `invalid_incident` for another `state` |
| `GET` | `/api/incidents/{id}` | `ViewData` | none | `incident`, `events` (oldest first: `at`, `kind` (`opened`, `changed`, `acknowledged`, `reminded`, `escalated`, `note`, `closed`), `username` (`null`: the hub), `text`). Clients get no `note` events and no acknowledgement text. 404 outside the user's companies |
| `POST` | `/api/incidents/{id}/acknowledge` | `Silence` | `{"text": "On my way"}` (may be empty) | The incident. 409 `incident_conflict` if it is closed or acknowledged already; 400 `invalid_incident` (over 2000 characters) |
| `POST` | `/api/incidents/{id}/notes` | `Silence` | `{"text": "…"}` | The incident. 400 `invalid_incident` (empty, over 2000 characters) |

## Alerts: channels and routing

Where alerts go (M3-03, D-66; how: [guide/alerting.md](../guide/alerting.md#channels)). Secrets are write-only: a request may set one; no answer contains one. Audited as `alerts.settings`, `channel.create`, `channel.update`, `channel.delete`, `alerts.routes` (never with a secret).

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/admin/alerts` | `ViewAdmin` | none | `settings` (as below, `smtp_password` `null`, `smtp_password_set`), `channels` (each: `id`, `name`, `kind`, `enabled`, `target`, `secret_set`, `last_delivery_at`, `last_error`), `rules`, `companies` (for the rule editor: `company_id`, `company_name`, `sites` as `[id, name]`), `digest` (`hour`, `timezone`, `channels`), `digest_last_sent` (the local date it last went) |
| `POST` | `/api/admin/alerts/settings` | `ManageData` | `{"hub_url", "smtp_server", "smtp_port", "smtp_security", "smtp_username", "smtp_password", "smtp_from"}` | `{"success": true}`. `smtp_security`: `starttls` (default), `tls`, `none`. `smtp_password`: left out or `null` keeps it, `""` clears it. 400 `invalid_alerts` (a hub address not `http(s)://`, a mail server without a From address, …) |
| `POST` | `/api/admin/alerts/channels` | `ManageData` | `{"name", "kind", "enabled", "target", "secret"}` | The channel. 400 `invalid_alerts` with the reason |
| `POST` | `/api/admin/alerts/channels/{id}` | `ManageData` | as above; `secret` left out or `null` keeps it (a new `kind` needs a new one) | The channel; 404 if none |
| `POST` | `/api/admin/alerts/channels/{id}/delete` | `ManageData` | none | `{"success": true}`. It leaves every rule; a rule left with no channel is removed |
| `POST` | `/api/admin/alerts/channels/{id}/test` | `ManageData` | none | Sends a made-up alert (`event` `test`) now, once: `{"ok", "error"}` (`error`: why it failed, never with a URL). Recorded as the channel's last delivery |
| `POST` | `/api/admin/alerts/digest` | `ManageData` | `{"hour": 7, "timezone": "Europe/Madrid", "channels": [1]}` | The daily digest (D-71): an hour 0–23, local to an IANA zone, to e-mail channels (none: off). `{"success": true}`; 400 `invalid_alerts` (not a time zone, a channel that isn't e-mail, an hour over 23) |
| `POST` | `/api/admin/alerts/digest/send` | `ManageData` | none | Sends the digest now (not counted as the day's): `{"ok", "error"}`; 400 `invalid_alerts` without channels |
| `POST` | `/api/admin/alerts/routes` | `ManageData` | `{"rules": [{"name", "companies", "sites", "kinds", "check_kinds", "channels", "stop", "remind_every_minutes", "escalate_after_minutes", "escalate_to"}]}` | Replaces every rule, in this order. `sites` are `company/site`; `kinds` `down`, `warning`, `recovered`, `probe_expired`; empty lists match all. `remind_every_minutes` (15–10080) and `escalate_after_minutes` (5–1440, with `escalate_to`: channel IDs) are optional (D-69). 400 `invalid_alerts` (no channel, an unknown channel, a time out of range, an escalation without channels or time) |

`kind` and what `target` / `secret` are: `email` (addresses, comma-separated / none), `webhook` (its URL / the signing key), `teams`, `slack`, `discord` (none / the webhook URL), `telegram` (the chat ID / the bot token), `ntfy` (the topic URL, `https://<server>/<topic>` / an access token, optional, D-70). A webhook's body and signature: [guide/alerting.md](../guide/alerting.md#webhook-signatures).

## Probe releases and updates

Signed probe releases on the hub, and how they are rolled out: canaries first, pause, each company's policy and window (M2-15, D-55; M4-01, D-74; how it works: [install/probe.md](../install/probe.md#updates)). Audited as `release.add`, `release.approve`, `release.remove`, `rollout.pause`, `rollout.resume`, `rollout.promote` (the last two also by the hub itself), `probe.canary`, `company.update_policy`.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/admin/probe-updates` | `ViewAdmin` | none | `rollout` (`null`: none approved; else `version`, `approved_by`, `approved_at`, `stage` (`canary` or `all`), `soak_minutes`, `canaries` and `canaries_ready` (canaries following it, and how many run it), `promote_at` (when it goes to every probe on its own), `promoted_at`, `promoted_by` (`null` with `promoted_at`: the hub), `paused` (`null`, or `at`, `by` (`null`: the hub), `reason`)), `releases` (newest first: `version`, `released`, `protocol`, `platforms` such as `"linux x86_64"`, `rollback_from`: the versions its signed rollback lets go back to it), `has_release_key` (`false`: the hub was built without one and takes no release), `probes` (as `GET /api/probes`, every company, plus `canary` and `update_status`: `{"state": "up_to_date"}`, `updating`, `none_approved`, `company_off`, `no_version`, `revoked`, `paused`, `awaiting_canaries`, `outside_window` (with `window`), `refused`, `missing`, `no_rollback`, most with the `version`), `companies` (not archived: `company_id`, `company_name`, `policy`) |
| `POST` | `/api/admin/probe-updates/approve` | `ManageData` | `{"version": "0.2.0", "soak_minutes": 60}` (`soak_minutes` optional, 0 to 10080), or `{"version": null}` to stop | Rolls it out: to the canaries first if there are any, then every probe `soak_minutes` after every canary runs it; with none, to every probe at once. A probe gets it in the answer to its next report. 400 `invalid` (soak too long); 404 if the hub has no such release. `{"success": true}` |
| `POST` | `/api/admin/probe-updates/pause` | `ManageData` | none | Offer it to no more probes. 404 if nothing is rolled out; 409 `rollout_state` if already paused |
| `POST` | `/api/admin/probe-updates/resume` | `ManageData` | none | Offer it again. 404, or 409 `rollout_state` if not paused |
| `POST` | `/api/admin/probe-updates/promote` | `ManageData` | none | To every probe now (and resumed). 404, or 409 `rollout_state` if every probe is already offered it |
| `POST` | `/api/admin/probe-updates/probes/{id}/canary` | `ManageData` | `{"canary": true}` | Make the probe a canary, or not. 404 if no such probe |
| `POST` | `/api/admin/probe-updates/companies/{id}` | `ManageData` | `{"mode": "follow"}` (the default), `{"mode": "off"}` or `{"mode": "pin", "version": "0.2.0"}`; optionally `"window": {"start": "02:00", "end": "05:00"}` (local time at each probe's site; past midnight if `end` is earlier) | The company's update policy. 400 `invalid` (a pin without a version, a version without a pin, a bad or empty window); 404 (no such company, or no such release) |
| `POST` | `/api/admin/probe-releases` | `ManageData` | the release `.tar.gz` (`blentinelmake release build`) as the raw body, up to 256 MB; or a rollback's (`blentinelmake release rollback`) | Adds it: `{"version", "released", "protocol", "platforms", "rollback_from"}`. Adding the same release again changes nothing but its rollback; a rollback alone is added to the release on the hub it is for (404 if there is none). 400 `untrusted_release` (not signed by a release key the hub trusts), `invalid_release` (not an archive, a file missing or not the one the manifest signs, a rollback for another release), `unsupported_protocol_version`; 409 `release_exists` (that version, built differently) |
| `POST` | `/api/admin/probe-releases/{version}/delete` | `ManageData` | none | Deletes it. 409 `release_approved` for the one rolled out or one a company is pinned to. `{"success": true}` |
| `GET` | `/download/releases/{version}/{file}` | none (also on the plain HTTP port) | none | A binary of a release, for a probe updating itself; `file` must be one the manifest lists. 404 otherwise |

## Hub keys

What probes pin of the hub, and rotating its certificate and key (M2-13, D-61; how it works: [install/hub.md](../install/hub.md#rotating-the-certificate-and-key)). Audited as `rotation.start`, `rotation.switch` (by the hub itself when every probe is ready), `rotation.cancel`.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/admin/hub-keys` | `ViewAdmin` | none | `hub_key` (hex), `tls_enabled`, `tls_spki_sha256` (hex of the certificate's public-key pin, or `null`), `tls_expires`, `rotation` (the one in progress, or `null`), `history` (the last 5, newest first). A rotation: `id`, `rotates_tls`, `rotates_key`, `window_days`, `state` (`announcing`, `switched`, `cancelled`), `started_by`, `started_at`, `deadline_at`, `ended_by` (`null` when the hub switched by itself), `ended_at`, `overdue`, `ready` (probes holding the next pins), `waiting` (`id`, `name`, `company_name`, `probe_version`, `last_seen_at`, `reason`) |
| `POST` | `/api/admin/hub-keys/rotation` | `ManageData` | `{"tls": true, "key": true, "window_days": 14}` | Starts it: the rotation as above. 400 `invalid_rotation` (neither chosen, window not 1–90, `tls` on a hub without HTTPS); 409 `rotation_in_progress` |
| `POST` | `/api/admin/hub-keys/rotation/switch` | `ManageData` | none | Switches now: `{"success": true, "probes_not_ready", "codes_revoked"}`. Probes not ready must enroll again. 404 if none is in progress |
| `POST` | `/api/admin/hub-keys/rotation/cancel` | `ManageData` | none | `{"success": true}`; the new files are deleted. 404 if none is in progress |

## Thresholds

When a check counts as Warning or Down (M3-01, D-64; how: [guide/alerting.md](../guide/alerting.md#thresholds)). A body is `{"cpu": …, "memory": …, "disk": …, "latency": …}`, each `{"warning": …, "down": …}` with a number, `"off"`, or left out (inherited). An answer is `{"own": <that>, "inherited": [["disk", {"warning": {"value": 90, "from": "built_in"}, "down": {…}}], …]}`: what this level sets, and what applies where it sets nothing. A check's own limits are part of the check (`limits`). Audited as `thresholds.update`.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/admin/thresholds` | `ViewAdmin` | none | The hub's default |
| `POST` | `/api/admin/thresholds` | `ManageData` | thresholds | `{"success": true}`. 400 `invalid_thresholds` (warning not below down, a percent over 100, a negative or huge latency) |
| `GET` | `/api/companies/{id}/thresholds` | `ManageCompanies` | none | The company's (404 outside the user's companies) |
| `POST` | `/api/companies/{id}/thresholds` | `ManageCompanies` | thresholds | As above |
| `GET` | `/api/companies/{id}/sites/{site}/thresholds` | `ManageCompanies` | none | A site's |
| `POST` | `/api/companies/{id}/sites/{site}/thresholds` | `ManageCompanies` | thresholds | As above |

## HTTPS certificate

The hub's certificate from Let's Encrypt or another ACME authority (M2-14, D-62; how it works: [install/https.md](../install/https.md#a-certificate-from-lets-encrypt)). Always issued for the hub's own TLS key, so what probes pin doesn't change. Audited as `certificate.settings`, `certificate.issue` and `certificate.issue_failed` (by the hub itself when it renews).

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/admin/certificate` | `ViewAdmin` | none | `tls_enabled`, `settings` (as below), `certificate` and `next_certificate` (a rotation's, or `null`): `issuer`, `names`, `self_signed`, `not_before`, `not_after`; `next_attempt_at` (`null`: off or nothing to do), `last_attempt_at`, `last_error` (`null` if the last attempt worked), `last_issued_at`, `https_port`, `http_port` (`null`: HTTPS only) |
| `POST` | `/api/admin/certificate` | `ManageData` | `{"enabled": true, "domains": ["hub.example.com"], "email": "", "authority": "lets_encrypt", "directory_url": "", "ca_pem": "", "challenge": "tls_alpn", "terms_agreed": true}` | Saves them; switched on, new names or authority, or saved while the last attempt had failed: asks for the certificate at once and answers when done: `{"success": true, "error": null}`, or `error` with the authority's reason (the settings stay saved; the hub tries again later). Switched off: back to a self-signed certificate with the same key. `authority`: `lets_encrypt`, `lets_encrypt_staging`, `custom` (then `directory_url`, `https://…`, and `ca_pem`, its root if not public); `challenge`: `tls_alpn` (port 443) or `http` (port 80). 400 `invalid_certificate_settings` (no names, not DNS names, more than 10, terms not accepted, the hub doesn't serve HTTPS itself, `http` on an HTTPS-only hub) |
| `POST` | `/api/admin/certificate/renew` | `ManageData` | none | Asks now, whatever the certificate's age and past failures. `{"success": true}`; 502 `certificate_not_issued` with the authority's reason; 409 `acme_off` |
| `GET` | `/.well-known/acme-challenge/{token}` | none (also on the plain HTTP port) | none | The answer to an HTTP-01 challenge while an order waits (`text/plain`); 404 otherwise |

## Backups

The hub's own backups (M2-12, D-60; how they work: [install/hub.md](../install/hub.md#backups)). Audited as `backup.create`, `backup.delete`, `backup.schedule`, `backup.download`, `backup.download_failed`. Each backup holds the hub's private key, so downloading one takes the user's password again.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/admin/backups` | `ViewAdmin` | none | `schedule` (`enabled`, `hour_utc`, `keep`), `folder` (on the hub's server), `next_at` (`null`: schedule off), `last_run_at` and `last_error` (the last scheduled backup; `last_error` `null` if it worked), `backups` (newest first: `name`, `kind` = `manual`, `scheduled` or `pre_upgrade`, `created_at`, `size_bytes`) |
| `POST` | `/api/admin/backups` | `ManageData` | none | Backs up now; answers when the file is written: `{"name", "kind": "manual", "created_at", "size_bytes"}`. 409 `backup_exists` if one was made the same second |
| `POST` | `/api/admin/backups/schedule` | `ManageData` | `{"enabled": true, "hour_utc": 3, "keep": 7}` | `{"success": true}`. 400 `invalid_schedule` (hour not 0–23, keep not 1–365) |
| `POST` | `/api/admin/backups/{name}/delete` | `ManageData` | none | Deletes that backup file. 404 if there's no backup of that name (only hub backup names are accepted). `{"success": true}` |
| `POST` | `/api/admin/backups/{name}/download` | `ManageData` | `{"password": "…"}` (your own) | `{"url": "/api/admin/backups/download/<token>"}`: a link that works once, for 60 s, with this session's user. 400 `wrong_password` (counts like a failed login); 429 `too_many_attempts`; 404 if no such backup |
| `GET` | `/api/admin/backups/download/{token}` | `ManageData` | none | The `.tar.gz` (`Content-Disposition: attachment`), streamed. 404 if the link expired, was used, or is another user's |

## Admin

Listings, storage info and the archive list need `ViewAdmin`; deleting and archiving need `ManageData`.

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/admin/companies` | none | All company IDs |
| `GET` | `/api/admin/probes` | none | All probes: `probe_id`, `company_id`, `status`, `last_seen_at`, `first_seen_at` |
| `POST` | `/api/admin/delete-company-data` | `{"company_id"}` | Deletes the company's reports (its probes stay known). `{"success": true}` |
| `POST` | `/api/admin/delete-probe-data` | `{"probe_id"}` | Deletes the probe's reports (the probe stays known). `{"success": true}` |
| `POST` | `/api/admin/remove-probe` | `{"probe_id"}` | Forgets the probe completely, its enrollment included: it must enroll again to report. `{"success": true}` |
| `GET` | `/api/admin/storage-info` | none | `current_db_size_mb`, `warn_threshold_mb`, `retention_days`, `last_archive_date`, `last_archive_size_mb`, `warning` |
| `GET` | `/api/admin/archives` | none | Archives, newest first |
| `POST` | `/api/admin/archive` | `{"cutoff_days"}` (optional) | Archives reports older than that many days (default: `archive_older_than_days`). `{"success": true, "filename", "size_mb"}` |

## Archive viewer (read-only)

The same shapes as the live endpoints, read from archive `{archive_id}`; `ViewData`. 404 (`not_found`) if the archive doesn't exist or can't be opened:

- `GET /api/archive/{archive_id}/companies`
- `GET /api/archive/{archive_id}/company/{company_id}/probes`
- `GET /api/archive/{archive_id}/company/{company_id}/uptime?range=24h|7d|30d|all`: as the live endpoint, but the range ends at the archive's newest report instead of now
- `GET /api/archive/{archive_id}/probe/{probe_id}/devices`
