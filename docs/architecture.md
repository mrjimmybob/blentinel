# Architecture

Blentinel (the **Bl**ind S**entinel**) is a multi-tenant monitoring platform for Managed Service Providers (MSPs). A central **hub** monitors many client networks, including networks behind NAT or strict firewalls, through **probes** that run inside those networks and **push** results to the hub.

The rules every design must follow are in [PRINCIPLES.md](PRINCIPLES.md). This page describes how the system is built today.

## Components

```
 client network A                  client network B
 +--------------------+            +--------------------+
 | probe --> devices  |            | probe --> devices  |
 +---------+----------+            +---------+----------+
           |  signed + sealed reports        |   (outbound only: the hub
           +----------------+----------------+    never connects to a probe)
                            v
                  +-------------------+        +----------------+
                  |        hub        | <----> |  web browser   |
                  |  API / UI / DB    |        | (technicians)  |
                  +-------------------+        +----------------+
```

The workspace has four crates:

| Crate | What it is |
|---|---|
| `probe` | The agent. A single self-contained binary inside a client network. It runs checks on an interval, signs and seals each report, pushes it to the hub, and buffers reports while the hub is unreachable. |
| `hub` | The server. It authenticates and stores reports, tracks probe liveness, sends alerts, and serves the web UI (Leptos, server-rendered and hydrated with WebAssembly) and a JSON API. |
| `common` | Shared by both: report models, protocol constants and rules (`common::protocol`), and the sealing/signing code (`common::crypto`), so both sides use exactly one implementation of the wire format. Also the plumbing both binaries need: key files and owner-only secrets (`common::fs`), config-file watching for hot reload (`common::watch`), and the command-line flags they share (`common::cli`). |
| `blentinelmake` | Developer build and packaging tool. |

## Probe

Each cycle (every `interval` seconds) the probe:

1. Runs its configured checks in parallel (ping, HTTP, TCP, local CPU/memory/disk/uptime).
2. Builds a `StatusReport` with the results and a timestamp.
3. **Signs** it with its Ed25519 identity key and **seals** it for the hub (see [security.md](security.md)).
4. Hands it to the **outbox** (`probe/src/outbox.rs`):
   - Nothing waiting: send it directly. Nothing is written to disk unless delivery fails transiently.
   - A backlog exists: queue it behind the backlog and deliver oldest-first. The hub only accepts reports newer than the last one it stored.
   - Delivered (200) or already stored (409): deleted. Permanently rejected (other 4xx): dropped and logged. Hub unreachable or failing (network error, timeout, 5xx): kept and retried.
   - The buffer (`blentinel_blackbox.db`) is capped at 7 days and 20,000 reports.

All probe state lives in its own directory (PRINCIPLES FP-2).

## Hub

The hub is layered (ENGINEERING §2.2):

```
HTTP handlers    hub/src/api.rs, ingest.rs, archive_viewer.rs
     |           thin: parse, authorise, map results to HTTP
     v
Services         hub/src/service/  (business rules; no HTTP, no SQL)
     |             ingest     accept a report (open, verify, time window, store, alerts)
     |             alerts     when to alert (Up/Down, thresholds, expiry) via a Notifier
     |             liveness   mark silent probes expired and alert on them
     |             retention  database size warnings and archiving
     |             silences   silencing a resource
     |             dashboard  views that combine storage with configuration
     v
Storage traits   hub/src/storage/mod.rs
     |           ReportStore, ProbeStore, AlertStore, ArchiveStore
     v
SQLite backend   hub/src/storage/sqlite/  (the only place with SQL)
                 schema: hub/migrations/sqlite/
```

Types shared by the server and the browser (every JSON shape, the `Severity` rule, resource keys) are defined once in `hub/src/model.rs`.

### Accepting a report

`POST /api/report` → `service::ingest`, in this order:

1. **Open the seal** with the hub's X25519 key → 401 if it can't be opened.
2. **Parse** the report and check its **protocol version** → 400.
3. The probe must be **enrolled** (a probe record, created by `POST /api/enroll` with a code) and not revoked, and the report must carry a **valid signature** from it → 403. The report is filed under the company and site in the record, whatever it says.
4. The **timestamp** must be within the accepted window: at most 5 min in the future, at most 7 days old → 422.
5. **Store** it only if it is newer than the probe's latest stored report (one atomic statement) → 409 otherwise.
6. Refresh the probe's **heartbeat**, re-arm its "offline" alert, and evaluate **alerts** in the background.

The response carries the hub's protocol version in the `X-Blentinel-Protocol` header. Full details are in [reference/protocol.md](reference/protocol.md).

### Severity

The dashboard's status for probes and companies is computed on the server (`model::Severity`):

| Level | Probe | Company |
|---|---|---|
| **critical** | active, with at least one device down | any *active* probe has a device down |
| **degraded** | expired (stopped reporting; its data is stale) | otherwise, any probe expired |
| **healthy** | active, nothing down | everything else |

A company is as bad as its worst probe. A live outage always shows as critical, even if another probe has expired.

### Background tasks

| Task | Every | Service |
|---|---|---|
| Probe expiry sweep (+ "probe offline" alerts) | 10 s | `service::liveness` |
| Database size check (+ auto-archive if enabled) | 10 min | `service::retention` |
| Removal of ended login sessions | 1 h | `service::users` |
| Config file watcher (hot reload) | on change | `hot_reload` |
| Alert evaluation for each accepted report | per report | `service::alerts` |

### Web interface

Leptos 0.8, rendered on the server and hydrated in the browser with WebAssembly. Pages: setup (first Owner), login, dashboard (companies by severity), company (probes, uptime chart), probe devices, admin (storage, archives, data deletion), archive viewer, companies (list and details with sites), users, audit log, my account. Every API route checks the user's permission (`routes.rs` → `auth::enforce`); the UI hides what the role can't do. The UI polls the JSON API every 15 s. It shows severity; it doesn't compute it.

### Data

The hub stores everything in one SQLite file (WAL mode). The schema is defined by the versioned migrations in `hub/migrations/sqlite/`, applied automatically at startup. See [reference/database.md](reference/database.md). Business code reaches it only through the storage traits, so other databases can be added later as optional backends (proposal 0011).

## Technology

| Area | Technology |
|---|---|
| Language | Rust (edition 2024), one Cargo workspace |
| Probe | Tokio, reqwest (rustls), surge-ping, sysinfo, bundled SQLite |
| Hub | Tokio, Axum, Leptos 0.8, SQLx (SQLite), rcgen/rustls (TLS), lettre (SMTP) |
| Crypto | X25519 + ChaCha20-Poly1305 (sealing), Ed25519 (signatures) |
