# 0008 · Codebase Health — Clean Code, DRY, Tests, CI

| | |
|---|---|
| **Status** | Proposed |
| **Milestone** | M0 — Foundations (must precede M1+) |
| **Priority** | P0 |
| **Decisions** | D-07, D-18, D-19, D-21 |
| **Standards** | [`ENGINEERING.md`](../ENGINEERING.md) |

## 1. Why first

M1–M3 change the database schema, the protocol, and auth on every endpoint. Doing that **without migrations, without tests, and with duplicated logic** means every change risks silent breakage. M0 is the safety net that makes the rest cheap.

## 2. Findings (from reviewing the code on 2026-09-27)

| # | Finding | Where | Principle |
|---|---|---|---|
| 1 | ~~**1 test in the whole workspace**~~ **Fixed (M0-04):** 63 tests + `known_issue_*` tests for open bugs; see ENGINEERING §4 | `common/src/lib.rs` | Tests |
| 2 | `Cargo.lock` is git-ignored → non-reproducible builds | `.gitignore` | Engineering |
| 3 | *(crypto consolidated into `common::crypto` in M0-04)* Five modules duplicated between probe and hub: `args` (94/100 lines), `crypto` (51/39), `hot_reload` (80/148), `identity` (19/90), `tls` (21/135) | `probe/src/*`, `hub/src/*` | DRY |
| 4 | **Fixed (M0-08):** `hub/src/model.rs`. API DTOs defined twice (`DashboardCompany`, `CompanyProbe`, `ProbeDevice`, `UptimeBucket`, `AdminProbe`, `ArchiveMetadata`, `StorageInfo`, `AlertSilence`) | `hub/src/db.rs` **and** `hub/src/app.rs` | DRY |
| 5 | **Fixed (M0-08, D-31):** one server-side rule. Severity rule written in UI code only (`company_severity_rank`, `ProbeRow`), and with **two different orderings** (sort puts devices-down first; the card color puts expired first) | `hub/src/app.rs` | DRY / single source |
| 6 | **Fixed (M0-08):** `model::resource_key`. Resource key `format!("{}:{}:{}:{}")` built in several places | `app.rs`, alerts | DRY / types |
| 7 | `app.rs` is 1,960 lines with all pages and components in one file | `hub/src/app.rs` | Clean code |
| 8 | Handlers call SQL directly; business rules spread over handlers, UI, and alerts | `hub/src/api.rs`, `db.rs` | Layering |
| 9 | Schema via `CREATE TABLE IF NOT EXISTS`, no migrations, no FK enforcement | `hub/src/db.rs::setup_tables` | Engineering |
| 10 | Status stored as `format!("{:?}", Health)` strings | `db.rs`, `alerts/engine.rs` | Types |
| 11 | `println!`/`eprintln!` logging in the hub; `tracing` only in the probe | hub | Consistency |
| 12 | Many `.unwrap()`s in UI and handler code | `app.rs`, `main.rs` | Errors |
| 13 | Crates on different editions (probe 2024, hub 2021) and major versions (`thiserror` 1 vs 2) | `Cargo.toml`s | Consistency |
| 14 | No `protocol_version` despite a comment requiring one | `common/src/models.rs` | Protocol |
| 15 | Leftover Leptos starter `hub/README.md`; Playwright `example.spec.ts` is the template test | `hub/` | Clean code |
| 16 | CI (`.woodpecker.yml`) only builds the hub; no fmt, clippy, or tests | root | CI |
| 17 | Secret key files (`identity.key`, `hub_identity.key`, `hub_auth.token`) sit untracked in the repo root **without** `.gitignore` rules. One `git add .` would commit them. | root | Security |
| 18 | No root `LICENSE` (only `hub/LICENSE`), no `CONTRIBUTING.md`, no `CHANGELOG.md` | root | Docs |

## 3. Work items (in order)

1. **Hygiene (S):** commit `Cargo.lock`; ignore `*.key`, `*.token`, `blentinel_*.toml` (except examples), `publish/`, `manual/*.{aux,log,out,toc,pdf}`; root `LICENSE`; `CHANGELOG.md`; `CONTRIBUTING.md`.
2. **Workspace consistency (S):** `[workspace.package]` (version, edition 2024, license) and `[workspace.dependencies]`; align versions.
3. **CI (M), ON HOLD (D-24, owner-controlled):** the GitHub Actions pipeline from ENGINEERING §5. Delete `.woodpecker.yml` once green (D-19). CI calls `blentinelmake` (D-21).
4. **Test harness (M):** hub `Router` builder usable from tests (`build_app(state) -> Router`), temp-DB fixture, `Clock` trait, probe transport pointed at the in-process hub. First tests: current behaviour of dashboard aggregation, uptime bucketing, silence round-trip, report verify/decrypt (tampered → rejected). **These characterisation tests lock in behaviour before the refactors.**
5. **Migrations (M):** introduce `sqlx::migrate!`. Migration `0001_baseline.sql` equals today's schema (existing DBs are marked as at-baseline). Turn on foreign keys.
6. **Protocol version (S):** add `protocol_version` to report and ack (D-18). The hub accepts reports without it as v1.
7. **Consolidate duplicates into `common` (M):** crypto, identity, TLS pinning, arg-parsing helpers, hot-reload helper, domain rules (severity, status enum, resource/check identity), shared DTO module compiled for `ssr` + `hydrate`.
8. **Hub layering + storage traits (L, incremental):** `api/` → `service/` → `storage::Storage` traits (SQLite implementation; see [0011](0011-storage-backends.md)), one area at a time (dashboard, company, probe, silences, admin), each with tests before and after.
9. **Split `app.rs` (M):** `pages/{dashboard,company,probe,admin,archive,login}.rs`, `components/{company_card,probe_row,uptime_chart,silence_modal,header}.rs`, `api_client.rs`.
10. **Errors and logging (M):** `ApiError` with JSON body; `tracing` in the hub; remove non-test `unwrap()`s.
11. **Docs cleanup (S–M):** see [0009](0009-documentation.md).

## 4. Exit criteria for M0

- fmt, clippy `-D warnings`, and tests pass on Linux and Windows (locally until CI is enabled, D-24).
- ≥ 40 meaningful tests covering: protocol verify/decrypt, dashboard/company/probe queries, uptime bucketing, severity rules, alert state, silence API, config validation.
- Migrations in place, `Cargo.lock` committed, secrets ignored, no duplicated modules between probe and hub.
- README links all resolve (link check, local tool until CI is enabled).
