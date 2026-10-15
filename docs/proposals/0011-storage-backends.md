# 0011 · Modular Storage: Embedded by Default, Pluggable Backends

| | |
|---|---|
| **Status** | Traits + SQLite backend + conformance suite **implemented** (M0-14, 2026-09-27; as hub modules per D-32). Backends beyond SQLite are future specs. |
| **Milestone** | M0 (storage traits + conformance tests) · M7 (optional backends) |
| **Priority** | P1 (abstraction) · P2 (PostgreSQL) · P3 (others) |
| **Depends on** | M0-05 (migrations), M0-09 (hub layering) |
| **Decisions** | D-22 (supersedes D-10), PP-3 (revised) |

## 1. Goal

- **Default:** the hub stores everything in an **embedded** database. Nothing to install, one file to back up (PP-1, PP-3).
- **Optional:** an admin can point the hub at an **external** database (PostgreSQL first, later others) for scale, high availability, or company policy ("all data must live in our SQL Server").
- **Modular:** business logic never knows which database is underneath. Adding a backend means adding one module that passes a shared test suite. Nothing else changes.

## 2. Is SQLite the right embedded default? (evaluation)

**Expected load:** 100 probes × 20 checks × 1 result every 60 s ≈ **33 inserts/s**, plus rollups. SQLite in WAL mode handles thousands of inserts per second on commodity hardware, so the embedded default has **two orders of magnitude of headroom** for the MSP segment.

| Embedded option | Verdict |
|---|---|
| **SQLite** (bundled via sqlx, already used) | ✅ **Keep as default.** Proven, zero-install, single-file backups, full SQL, and supported by the same library as PostgreSQL/MySQL (sqlx), which keeps backends consistent. |
| libSQL (SQLite fork) | 👀 Watch. SQLite-compatible, adds replication. Could become a drop-in upgrade later. |
| DuckDB | ❌ as primary. Excellent for analytics, but a C++ engine that adds ~20–30 MB, and single-writer semantics are a poor fit for continuous ingestion. Possible later as an *export/reporting* format. |
| redb / fjall / sled (key-value stores) | ❌ No SQL. All aggregations (dashboard, rollups, reports) would have to be hand-written. sled is effectively unmaintained. |
| SurrealDB embedded, Limbo (Rust SQLite rewrite) | ❌ for now. Too young for a monitoring system's primary store. |

## 3. Architecture

```
hub services (business rules)
        │   depend only on traits
        ▼
storage::Storage  ── trait (domain-level repositories)
   ├── CompanyRepo, SiteRepo, ProbeRepo, CheckRepo
   ├── ResultRepo      (ingest, rollups, time-bucketed queries, retention)
   ├── AlertRepo       (state, incidents, silences, maintenance)
   ├── UserRepo, SessionRepo, TokenRepo, AuditRepo
   └── SettingsRepo
        ▲
        │ implemented by
 ┌──────┴────────┬──────────────────┬───────────────────┬──────────────────┐
 storage-sqlite   storage-postgres   storage-mysql       storage-mssql       storage-oracle
 (default)        (feature)          (feature)           (feature)           (separate build)
```

- **Domain-level traits, not a generic SQL layer.** Each trait method is a business-level operation, e.g. `ResultRepo::uptime_buckets(company, range)`. Each backend implements it in its own dialect. This avoids lowest-common-denominator SQL and the performance problems of an ORM, and keeps SQL in exactly one place per backend.
- **Hot paths stay native per backend:** time bucketing (`strftime` vs `date_trunc` vs `DATEADD`), upserts (`ON CONFLICT` vs `MERGE`), JSON columns.
- **Types:** repositories accept and return domain types from `common`/`service` (`CompanyId`, `CheckStatus`, `DateTime<Utc>`), never driver rows.
- **Injection:** `AppState` holds `Arc<dyn Storage>`, built at startup from config. Async trait methods use `async-trait` (already a dependency) until native dyn-async traits are stable.
- **Migrations:** one migration folder per backend (`storage-sqlite/migrations`, `storage-postgres/migrations`, …) with the **same version numbers**. A schema-parity test asserts that every backend has the same set of migration versions.
- **Crates:** each backend is its own workspace crate behind a cargo feature, so an unused backend adds no code to the binary.

## 4. Configuration

```toml
[database]
# Default (embedded). Path is relative to the data directory.
url = "sqlite://blentinel.db"
# Optional external database (examples):
# url = "postgres://blentinel@db.internal/blentinel?sslmode=require"
# url = "mysql://blentinel@db.internal/blentinel"
# url = "mssql://blentinel@sql01.internal/blentinel"
max_connections = 10
```

- The password can be supplied via the `BLENTINEL_DATABASE_URL` environment variable, so it doesn't sit in the file.
- `hub install` writes the SQLite default, so a fresh install asks no database questions (PP-1).
- `hub doctor` tests the connection, the migration state, and permissions.

## 5. Backend matrix

| Backend | Driver | Pure Rust / static? | Release plan | Priority |
|---|---|---|---|---|
| **SQLite** | `sqlx` (bundled) | ✅ | Always in official builds | P0 (exists) |
| **PostgreSQL** | `sqlx` + rustls | ✅ | In official builds, selected at runtime | **P2** |
| **MySQL / MariaDB** | `sqlx` + rustls | ✅ | Official builds once it passes conformance | P3 |
| **SQL Server** | `tiberius` (sqlx dropped MSSQL in 0.7) | ✅ | Official builds once it passes conformance | P3 |
| **Oracle** | `oracle` crate (wraps OCI) | ❌ **needs Oracle Instant Client** (C libraries) on the hub machine | **Separate "enterprise" build flavour**, on demand only. It can't be in the default single binary without breaking PP-1. | P3 (on request) |

The probe is **not affected**. Its local offline queue stays embedded SQLite (FP-2, FP-4).

## 6. Data portability

- `hub db export <file>` / `hub db import <file>`: a backend-neutral dump through the `Storage` traits (versioned JSON Lines). This is also the logical backup format for external databases.
- `hub db migrate --to <url>`: move a running installation from SQLite to PostgreSQL (or between any two backends) with downtime of minutes, not hours. P3.
- Backups: SQLite keeps `hub backup` (file-level, consistent snapshot). For external databases, the docs point to native tooling (`pg_dump` etc.) plus the logical export.

## 7. Testing (DRY: one suite, all backends)

- **Conformance suite:** `storage::conformance::run_all(factory)` is a generic test set written once and run against every backend. It covers CRUD, scoping, time bucketing at DST boundaries, rollup correctness, retention, concurrency (parallel ingest), and migrations from the previous version.
- SQLite runs on every `cargo test`. External backends run when `BLENTINEL_TEST_<BACKEND>_URL` is set (locally via Docker; CI wiring is left to the owner).
- Schema-parity test (§3).

## 8. Phasing

1. **M0 (with M0-09 hub layering):** define the traits, move all SQL behind the SQLite implementation, write the conformance suite. *No user-visible change*, and almost no extra cost if done while layering.
2. **M7:** PostgreSQL backend → docs → release.
3. **M7 (on demand):** MySQL/MariaDB, SQL Server, Oracle flavour, `hub db migrate`.
4. **Watch:** libSQL as a drop-in SQLite replacement if replication is needed.

## 9. Principle check

| | |
|---|---|
| PP-1 / PP-3 | Embedded default: nothing to install. External DBs are **optional, never required**. ✔ |
| FP-1…FP-4 | Probe unaffected. ✔ |
| Oracle | Needs a native client. It is isolated in a separate build flavour so the default hub stays a single self-contained binary. ✔ |

## 10. Docs to update

`docs/install/hub.md` (database section: default + optional backends), `docs/reference/config.md` (`[database]`), `docs/architecture.md` (storage layer), `CONTRIBUTING.md` (how to add a backend: implement the traits, add migrations, pass conformance), CHANGELOG.
