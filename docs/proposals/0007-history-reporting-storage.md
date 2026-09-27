# 0007 · History, Reporting, Live UI & Storage

| | |
|---|---|
| **Status** | Proposed |
| **Milestone** | M6 — Insight & reporting |
| **Priority** | P1 (retention/rollups, per-check history, SLA report) · P2 (SSE live UI, search) · P3 (client status page) |
| **Depends on** | 0003 (`CheckId`, multi-metric results), 0005 (maintenance windows for SLA) |
| **Decisions** | D-12, D-20, D-22 |

## 1. Problem

- Every check result from every probe is stored **forever** as a raw row. The database grows linearly, which is why the archive feature and the DB-size warning had to exist.
- The UI shows only the **latest** state per device, plus a company-level uptime line. All of the stored history (latency, CPU, disk trends) is invisible.
- MSPs need **monthly availability reports per client** to justify contracts. None exist.
- The UI polls every 15 s and has repeatedly suffered from flicker (D-12).

## 2. Storage model

| Tier | Contents | Default retention |
|---|---|---|
| **Raw results** | Every result as received | **14 days** |
| **5-minute rollups** | Per check: count up/warn/down/unknown, min/avg/max latency, min/avg/max per metric | 90 days |
| **Hourly rollups** | Same shape | 2 years |
| **Daily rollups** | Same shape | Forever (tiny) |
| **State changes / incidents** | Every status transition, incident timeline | Forever |

- Rollups are computed incrementally by a background task (idempotent, keyed on `(check_id, bucket_start)`). The uptime and chart queries choose the tier by range: this replaces today's `strftime` bucketing over raw rows.
- Retention is configurable in the UI (Settings → Data retention).
- **Storage-agnostic** (D-22, [0011](0011-storage-backends.md)): retention and rollups are implemented behind the storage traits, so they work on embedded SQLite (the default) and any optional backend. With rollups, 100 probes × 20 checks × 60 s stays at roughly a few hundred MB steady-state instead of growing forever.
- **Archives** (D-20): once rollups ship, archiving becomes an optional *export* (full raw data for a date range → a standalone SQLite file) for compliance. Revisit whether the in-app archive viewer stays.

## 3. UI features

### 3.1 Check detail page (P1)

- Status timeline (a colored band) with incidents and maintenance windows overlaid.
- Charts per metric (latency, loss, disk %, toner…), with a range picker (24 h / 7 d / 30 d / 1 y / custom) and hover values.
- Availability % for the range (maintenance excluded), MTTR, incident count.
- Recent results table (raw tier).

### 3.2 Probe host page (P1)

CPU, memory, and disk trends for the probe's own host, plus **disk-full forecast**: a linear regression over 14 days → "C: full in ~12 days" (becomes a Warning when below a threshold).

### 3.3 Dashboard improvements (P1)

- **"Problems" view first:** everything Down, Warning, or Unknown across all companies, grouped by company › site, with ack state.
- Group by site inside companies. Search box (companies, sites, probes, checks, targets). Filters by status and tag.
- Upcoming TLS cert expiries and backup-freshness warnings widget.

### 3.4 Reports (P1)

- **Monthly availability report per company:** per-site and per-check availability %, incidents with duration and root cause, maintenance performed, trend vs. previous month.
- Export **PDF** (rendered server-side, e.g. `typst` or HTML→PDF in pure Rust) and **CSV**.
- **Scheduled delivery** by e-mail to the company contacts (0002), e.g. on the 1st of every month.
- Client-role users (0001) can view and download their own company's reports.

### 3.5 Live updates via SSE (P2)

- `GET /api/events` (Server-Sent Events, authenticated, scoped to the user's companies) streams `status_changed`, `incident_opened/closed`, `probe_online/offline`, `config_applied` events.
- The UI applies deltas to its local state, so there is no periodic full refetch and nothing re-renders without a change. This is the structural fix for the flicker. Polling at 60 s stays as a fallback.
- FP-1 is unaffected: this is hub → browser, not hub → probe.

### 3.6 Client status page (P3)

An optional public or unlisted page per company showing selected checks as "operational / degraded / outage", with its own URL and no login. Off by default.

## 4. Tests

- Rollup correctness: raw → 5 min → hourly → daily equals direct aggregation over raw (a property test); idempotent re-runs; late-arriving buffered reports (FP-4) land in past buckets correctly.
- Retention deletes only expired tiers. SLA math excludes maintenance windows and handles DST.
- Forecast function unit tests. SSE: scoping (a Client never receives other companies' events), reconnect with `Last-Event-ID`.

## 5. Docs to update

`docs/guide/dashboard.md`, `docs/guide/reports.md`, `docs/guide/data-retention.md`, retire the archive docs if D-20 decides so, CHANGELOG.
