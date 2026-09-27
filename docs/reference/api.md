# HTTP API reference

The hub's JSON API, used by the web UI and available to scripts. The probe endpoints are in [protocol.md](protocol.md). The route table is `hub/src/routes.rs`; the JSON shapes are defined once in `hub/src/model.rs`.

## Authentication

| Method | Path | Body | Result |
|---|---|---|---|
| `POST` | `/api/login` | the admin token, as **plain text** | 200 + `Set-Cookie: blentinel_session=…` (HttpOnly, 24 h), or 401 |
| `POST` | `/api/logout` | none | Ends the session |

Every endpoint below needs the `blentinel_session` cookie, and returns **401** (`unauthorized`) without a valid one.

## Errors

Every error response, here and on the probe endpoint, has the same JSON body:

```json
{ "error": { "code": "not_found", "message": "archive 42 not found or unreadable" } }
```

`code` is stable, so scripts should match on it. `message` is for people and may change.

| Status | `code` | When |
|---|---|---|
| 401 | `unauthorized` | No valid session, or a wrong admin token at login |
| 400 / 415 / 422 | `invalid_body` | The request body isn't the expected JSON (`message` says what's wrong) |
| 404 | `not_found` | The archive doesn't exist or can't be opened |
| 500 | `internal` | A failure on the hub, e.g. the database. The details are in the hub log, never in the response. |

The probe endpoint's codes are in [protocol.md](protocol.md#responses-to-post-apireport). The type is `common::protocol::ErrorBody`, shared by the hub, the web UI and the probe.

## Dashboard

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

| Method | Path | Body | Result |
|---|---|---|---|
| `GET` | `/api/silences` | none | Silences that haven't expired |
| `POST` | `/api/silence` | `{"resource_key", "reason", "duration_hours"}` | `{"success": true, "silence_id": 1}` |
| `POST` | `/api/silence/delete` | `{"silence_id": 1}` | `{"success": true}` |

`resource_key` is `company_id:probe_id:resource_name:target`. `duration_hours` is `null` for "until removed or the resource recovers". Silencing an already-silenced resource replaces the silence. See [guide/alerting.md](../guide/alerting.md).

## Admin

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

The same shapes as the live endpoints, read from archive `{archive_id}`. 404 (`not_found`) if the archive doesn't exist or can't be opened:

- `GET /api/archive/{archive_id}/companies`
- `GET /api/archive/{archive_id}/company/{company_id}/probes`
- `GET /api/archive/{archive_id}/company/{company_id}/uptime` (known issue D-26: always empty)
- `GET /api/archive/{archive_id}/probe/{probe_id}/devices`
