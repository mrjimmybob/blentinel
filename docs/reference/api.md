# HTTP API reference

The hub's JSON API, used by the web UI and available to scripts. The probe endpoints are in [protocol.md](protocol.md). The route table is `hub/src/routes.rs`; the JSON shapes are defined once in `hub/src/model.rs`.

## Authentication and access

The API is served over **HTTPS** (port 3443 by default); on the plain-HTTP port only the probe endpoints answer, anything else is redirected there ([install/https.md](../install/https.md)).

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
| `GET` | `/api/probes` | `ViewData` | none | Enrolled probes: `id`, `name`, `company_id`, `company_name`, `site_id`, `site_name`, `state` (`active`/`revoked`), `hostname`, `os`, `arch`, `probe_version`, `enrolled_at`, `last_seen_at`, `interval_secs` |
| `POST` | `/api/probes/{id}` | `ManageCompanies` | `{"name", "site_id", "interval_secs"}` | Renames it, moves it to another site of its company, sets how often it reports (5–3600 s; it gets the new interval with its checks). `{"success": true}` |
| `POST` | `/api/probes/{id}/revoke` | `ManageData` | none | Its reports are refused (`403 probe_revoked`) until it enrolls again. `{"success": true}` |
| `GET` | `/api/enrollment-codes?company=` | `ManageCompanies` | none | Codes that can still be used (not expired, used up or revoked): `id`, `company_id`, `site_id`, `probe_name`, `created_by`, `created_at`, `expires_at`, `max_uses`, `uses`. `company` is optional |
| `POST` | `/api/enrollment-codes` | `ManageCompanies` | `{"company_id", "site_id", "hub_url", "probe_name"?, "max_uses"? (1–1000, default 1), "valid_hours"? (1–720, default 24)}` | `{"code": "BLT1-…", "info": {…as above}, "hub_url"}`. The code is in this answer only |
| `POST` | `/api/enrollment-codes/{id}/revoke` | `ManageCompanies` | none | `{"success": true}` |
| `GET` | `/api/probe-downloads` | `ManageCompanies` | none | The probe binaries this hub offers ([install/hub.md](../install/hub.md#probe-binaries)): `target`, `os`, `arch`, `size`, `sha256` (hex). The Add probe wizard builds its one-line commands from these |
| `GET` | `/download/probe/{target}` | none (also on the plain HTTP port) | none | The binary (`application/octet-stream`, header `x-checksum-sha256`). 404 if the hub has none for that target |

`hub_url` is the address probes will use (`http://` or `https://`). For an `https://` address the hub puts the fingerprint of its own certificate in the code, if it serves TLS. Invalid values: 400 `invalid`.

Probes themselves call `POST /api/enroll` (no session, no CSRF header; the body is sealed and signed): see [protocol.md](protocol.md#enrollment).

## Checks

M2-04, D-49. What each probe checks is a record on the hub; the probe gets its enabled checks and its interval in the answer to its next report ([protocol.md](protocol.md#the-probes-checks-config)). The kinds and their settings: [checks.md](checks.md). Limited to the user's company scope: another company's probe or check answers 404.

| Method | Path | Permission | Body | Result |
|---|---|---|---|---|
| `GET` | `/api/probes/{id}/checks` | `ViewData` | none | The probe's checks and whether it runs them (below) |
| `POST` | `/api/probes/{id}/checks` | `ManageCompanies` | `{"name", "spec", "enabled"? (default true)}` | The new check |
| `POST` | `/api/probes/{id}/checks/copy` | `ManageCompanies` | `{"from_probe_id"}` | Adds copies (new IDs, same names, specs and enabled flags) of every check of another probe in the user's companies. The new checks. 400 if it's the same probe or the copies would pass 500 |
| `POST` | `/api/checks/{id}` | `ManageCompanies` | `{"name", "spec", "enabled"? (default true)}` | Replaces name, spec and enabled flag; the ID stays, so its history, alert state and silence stay too. The check |
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

`sync` is `in_sync` (the probe runs `config_revision`), `pending` (it gets it with the answer to its next report) or `rejected` (it refused it; `config_error` says why). An invalid name or spec: 400 `invalid` with the reason (e.g. `ping needs an IP address, e.g. 192.168.1.1`); an unknown `kind`: 422 `invalid_body`. At most 500 checks per probe. Changes are in the audit log (`check.create`, `check.update`, `check.delete`, `check.copy`).

## Dashboard

All need `ViewData`.

### `GET /api/dashboard/companies`

One entry per company with probes, sorted by `company_id`. Archived companies are left out. `name` is the company's display name:

```json
[{
  "company_id": "example-company",
  "name": "Example Company Ltd",
  "total_probes": 2, "active_probes": 1, "expired_probes": 1,
  "devices_up": 5, "devices_down": 1,
  "severity": "critical",
  "last_report": "2026-03-10T12:01:00Z"
}]
```

`devices_up`/`devices_down` come from each probe's latest report, including stale ones from expired probes. `severity` (`critical` / `degraded` / `healthy`) is what to display; the rule is in [architecture.md](../architecture.md#severity).

### `GET /api/company/{company_id}/probes`

The company's probes, most recently seen first:

```json
[{
  "probe_id": "a773d2…", "probe_name": "head-office",
  "hostname": "office-pc-01", "site": "head-office",
  "status": "active", "severity": "healthy",
  "last_seen_at": "2026-03-10T12:00:30Z",
  "devices_up": 3, "devices_down": 0
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

### `GET /api/probe/{probe_id}/devices`

The results of the probe's latest report, one per check (checks deleted since are left out), sorted by name:

```json
[{
  "check_id": "9b77…3e", "name": "CPU", "kind": "cpu", "target": "localhost",
  "status": "Up", "message": "CPU usage: 23.4%", "latency_ms": null,
  "metrics": [{ "key": "usage", "value": 23.4, "unit": "%" }]
}]
```

The first metric is the headline value the UI shows.

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
| `GET` | `/api/silences` | none | Silences that haven't expired: `id`, `scope_type` (`check`), `scope_id` (the check ID), `company_id`, `reason`, `created_at`, `expires_at` |
| `POST` | `/api/silence` | `{"check_id", "reason", "duration_hours"}` | `{"success": true, "silence_id": 1}` |
| `POST` | `/api/silence/delete` | `{"silence_id": 1}` | `{"success": true}` |

A silence is on one check (D-49), so it survives renaming or retargeting the check. `duration_hours` is `null` for "until removed or the check recovers". Silencing an already-silenced check replaces the silence. An unknown check, or another company's: 404. See [guide/alerting.md](../guide/alerting.md).

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
