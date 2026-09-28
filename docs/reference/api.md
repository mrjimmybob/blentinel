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
| `ManageData`: delete data, remove probes, create archives | ✓ | ✓ | | |
| `ManageUsers`: users | ✓ | ✓ | | |

Without the permission: **403** `forbidden`. A user with a temporary password gets **403** `password_change_required` everywhere except `/api/auth/me`, `/api/me/password` and `/api/auth/logout`. The table is `hub/src/routes.rs`; `hub/tests/access.rs` checks every route against it.

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
  "role": "admin", "all_companies": true, "companies": [],
  "is_active": true, "must_change_password": false,
  "created_at": "2026-09-28T10:00:00Z", "last_login_at": "2026-09-28T10:05:00Z"
}
```

## Users

All need `ManageUsers`. Only an Owner may create, edit, reset or log out Owners (403 `forbidden`).

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/users` | none | All users, by username |
| `POST` | `/api/users` | `{"username", "display_name", "role", "email"?}` | `{"user", "temporary_password"}`: shown once; must be changed at first login. 409 `username_taken`, 400 `invalid_username` |
| `POST` | `/api/users/{id}` | `{"display_name", "role", "is_active", "email"?}` | The updated user. Deactivating ends their sessions. 409 `last_owner`; 403 when deactivating yourself |
| `POST` | `/api/users/{id}/reset-password` | none | `{"user", "temporary_password"}`; ends their sessions |
| `POST` | `/api/users/{id}/logout` | none | Ends all their sessions |

`role` is `owner`, `admin`, `technician` or `viewer`. `client`, and `"all_companies": false` with `companies`, are refused (400 `not_supported`) until company scoping exists (ROADMAP M1-04).

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
| 400 | `weak_password`, `wrong_password`, `invalid_username`, `not_supported` | See [Users](#users) |
| 409 | `username_taken`, `last_owner`, `already_set_up` | See [Users](#users) |
| 400 / 415 / 422 | `invalid_body` | The request body isn't the expected JSON (`message` says what's wrong) |
| 404 | `not_found` | The archive doesn't exist or can't be opened |
| 500 | `internal` | A failure on the hub, e.g. the database. The details are in the hub log, never in the response. |

The probe endpoint's codes are in [protocol.md](protocol.md#responses-to-post-apireport). The type is `common::protocol::ErrorBody`, shared by the hub, the web UI and the probe.

## Dashboard

All need `ViewData`.

### `GET /api/dashboard/companies`

One entry per company, sorted by `company_id`:

```json
[{
  "company_id": "example-company",
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

`status` is `active` or `expired`. `probe_name` comes from the hub's `[[probes]]` list and is empty if unnamed.

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

Devices from the probe's latest report, sorted by name:

```json
[{
  "name": "CPU", "resource_type": "local_cpu", "target": "localhost",
  "status": "Up", "message": "CPU usage: 23.4%",
  "latency_ms": null, "metric_value": 23.4, "metric_unit": "%"
}]
```

## Alert silences

Listing needs `ViewData`; creating and removing need `Silence`.

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/silences` | none | Silences that haven't expired |
| `POST` | `/api/silence` | `{"resource_key", "reason", "duration_hours"}` | `{"success": true, "silence_id": 1}` |
| `POST` | `/api/silence/delete` | `{"silence_id": 1}` | `{"success": true}` |

`resource_key` is `company_id:probe_id:resource_name:target`. `duration_hours` is `null` for "until removed or the resource recovers". Silencing an already-silenced resource replaces the silence. See [guide/alerting.md](../guide/alerting.md).

## Admin

Listings, storage info and the archive list need `ViewAdmin`; deleting and archiving need `ManageData`.

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/admin/companies` | none | All company IDs |
| `GET` | `/api/admin/probes` | none | All probes: `probe_id`, `company_id`, `status`, `last_seen_at`, `first_seen_at` |
| `POST` | `/api/admin/delete-company-data` | `{"company_id"}` | Deletes the company's reports (its probes stay known). `{"success": true}` |
| `POST` | `/api/admin/delete-probe-data` | `{"probe_id"}` | Deletes the probe's reports (the probe stays known). `{"success": true}` |
| `POST` | `/api/admin/remove-probe` | `{"probe_id"}` | Forgets the probe completely (it reappears if still whitelisted and reporting). `{"success": true}` |
| `GET` | `/api/admin/storage-info` | none | `current_db_size_mb`, `warn_threshold_mb`, `retention_days`, `last_archive_date`, `last_archive_size_mb`, `warning` |
| `GET` | `/api/admin/archives` | none | Archives, newest first |
| `POST` | `/api/admin/archive` | `{"cutoff_days"}` (optional) | Archives reports older than that many days (default: `archive_older_than_days`). `{"success": true, "filename", "size_mb"}` |

## Archive viewer (read-only)

The same shapes as the live endpoints, read from archive `{archive_id}`; `ViewData`. 404 (`not_found`) if the archive doesn't exist or can't be opened:

- `GET /api/archive/{archive_id}/companies`
- `GET /api/archive/{archive_id}/company/{company_id}/probes`
- `GET /api/archive/{archive_id}/company/{company_id}/uptime?range=24h|7d|30d|all`: as the live endpoint, but the range ends at the archive's newest report instead of now
- `GET /api/archive/{archive_id}/probe/{probe_id}/devices`
