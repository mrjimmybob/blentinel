# Blentinel — Engineering Standards

How we build Blentinel. This complements the Rust style rules in [`rust.instructions.md`](../rust.instructions.md), which remain in force, and is bound by [`PRINCIPLES.md`](PRINCIPLES.md).

---

## 1. Definition of Done

A change is done only when **all** of the following are true:

1. **Principle check passed.** Include the table from `PRINCIPLES.md` in the PR description.
2. **Tests added or updated** for the new or changed behaviour (see §4), and `cargo test-all` passes.
3. **Quality gates pass:** `cargo fmt --check`, `cargo lint` + `cargo lint-wasm` (clippy with `-D warnings`), `cargo test-all`, on Linux and Windows. The aliases are in `.cargo/config.toml`. Run them locally until CI is enabled; CI is owner-controlled (D-24).
4. **Docs updated in the same PR:** README, install guides, config reference, API reference, and anything else the change affects.
5. **`CHANGELOG.md` entry** under `Unreleased` (Keep a Changelog format).
6. **Migrations included** for any schema change (versioned, never edited after release).
7. **No new `unwrap()`/`expect()`** outside tests and truly unreachable cases. Any that remain are justified in a comment.

---

## 2. Architecture rules

### 2.1 Workspace layout (target)

```
common/        Shared protocol + domain: models, validation, crypto, identity, TLS pinning,
               status/severity rules, arg parsing helpers. No I/O frameworks.
probe/         Probe binary: checks, scheduler, transport, local queue, service integration.
hub/           Hub binary: HTTP API, services, alerting, UI (Leptos).
hub/src/storage/         storage::Storage traits (backend-neutral); sqlite/ = default embedded backend (D-32).
hub/tests/storage_conformance.rs  shared contract every backend must pass. Separate backend crates come with the 2nd backend (M7).
blentinelmake/ Developer build tool. CI calls the same commands (one build definition).
```

### 2.2 Hub layering

```
HTTP handlers (api/*)      thin: parse → authorize → call service → map result/error
        ↓
Services (service/*)       business rules: severity, alert state machine, enrollment, permissions
        ↓
Storage traits (storage::Storage)   domain-level repositories; no SQL types cross this line
        ↓ implemented by
Backends (storage-sqlite, …)        SQL only, in that backend's dialect; no business rules
```

- **No SQL in handlers or services. No HTTP types in services. No business rules in the UI or in backends.**
- Services (`hub/src/service/`) depend on the storage traits (`&dyn ProbeStore`, `Arc<dyn Storage>`, …), never on a concrete backend or driver type. They return typed results (e.g. `IngestError`); handlers map those to HTTP status codes.
- **Pure reads with no rules** (e.g. dashboard companies, probe devices) may go straight from handler to storage. Anything that applies a rule or combines sources goes through a service, so every caller (API, archive viewer, future mobile API) shares it.
- Rules that show up in several places are written **once** in `common`, `hub/src/model.rs` (shared with the UI) or `service`, and called from everywhere.

### 2.3 DRY: one definition of each thing

| Thing | Single source |
|---|---|
| Wire/API DTOs and shared UI/server rules | `hub/src/model.rs`, compiled for both `ssr` and `hydrate`: every JSON shape, `Severity`, `short_probe_id`; check specs and results come from `common::checks` |
| Probe↔hub protocol | `common::protocol` with an explicit `protocol_version` |
| Validation of checks/config | `common` (used by the hub UI, hub API **and** probe) |
| Crypto; key files; secret-file permissions; config-file watching (hot reload); common CLI flags | `common::crypto`, `common::fs`, `common::watch`, `common::cli`. TLS is not shared: the probe pins a certificate, the hub generates one (D-37). |
| Default config + its documentation | The generator behind `--init` is the source; docs embed its output |
| Build steps | `blentinelmake`; CI invokes it instead of re-implementing it |
| Dependency versions | `[workspace.dependencies]` in the root `Cargo.toml` |

### 2.4 Types over strings

- Status is an enum (`Up | Warning | Down | Unknown`), not a `format!("{:?}")` string.
- IDs are newtypes (`ProbeId`, `CompanyId`, `CheckId`), so a probe ID can't be passed where a company ID is expected.
- Resource identity is a stable `CheckId`, not the `"company:probe:name:target"` string.

### 2.5 Errors and logging

- Libraries/services: `thiserror` enums. Binaries/glue: `anyhow` with `.context()`.
- The HTTP API returns **JSON errors** `{ "error": { "code": "...", "message": "..." } }` via one `ApiError: IntoResponse` (`hub/src/error.rs`; the body type is `common::protocol::ErrorBody`, D-35). Handlers return `ApiResult<T>` and use `?`; request bodies use `ApiJson<T>`, so a bad body gets the same envelope. `ApiError::internal(context, cause)` logs the cause and never sends it.
- **`tracing` everywhere.** No `println!`/`eprintln!` outside CLI output. Structured fields (`probe_id`, `company_id`), not string concatenation. Levels: `error` = something failed, `warn` = rejected input or an operator should act, `info` = lifecycle events (startup, reloads, expiries, archiving), `debug` = per-request lines (`--verbose`), `trace` = payloads (`--debug`). Setup: `hub/src/logging.rs`, probe: `probe/src/logging.rs`.
- Never log secrets, tokens, password hashes, or decrypted payloads, except under the explicit `--debug` flag, and redacted even then.

### 2.6 Database

- **Embedded SQLite by default; external backends optional** (PP-3, D-22, proposal 0011).
- **Versioned migrations** (`sqlx::migrate!`, embedded at compile time). SQLite migrations live in `hub/migrations/sqlite/`, embedded by `storage::sqlite`. When other backends exist, each has its own folder with the **same version numbers**, enforced by a schema-parity test.
- **Changing the schema:** add `hub/migrations/sqlite/NNNN_description.sql` with the next number. **Never edit a committed migration.** `db::open` applies pending migrations at startup, and the test harness uses the same path, so `cargo test-all` exercises every migration.
- **Connection settings** (WAL, `foreign_keys`, busy timeout) are set in `db::connect` for **every** pooled connection, never as a one-off `PRAGMA` (SQLite applies them per connection).
- Every table has an explicit primary key and the indexes its queries use. Child rows use `ON DELETE CASCADE` where the parent owns them.
- Pre-alpha (D-30): databases that predate migrations are refused with instructions to delete them. No adoption or data conversion.
- Automatic **pre-migration backup** on hub upgrade (SQLite: file snapshot; external: logical export).
- A new backend is "supported" only when it passes the **shared conformance suite**.

### 2.7 Protocol compatibility

- Reports carry a signed `protocol_version` (current: **4**; absent = 1), which the hub reads before the rest of the report. The hub accepts the **current and previous** version (`MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION`), except across a change to what a report is: v4 is accepted alone (pre-alpha, D-30, D-49). The probe refuses to downgrade.
- Anything both sides must agree on (versions, header name, buffer age, clock skew) is a constant in **`common::protocol`**. Never duplicate these values.
- New signed fields are **appended** and omitted when absent (`skip_serializing_if`), so older reports keep their exact signed bytes. A unit test in `common/src/models.rs` pins the signed bytes.
- Breaking protocol changes require a `DECISIONS.md` entry and a release note.

### 2.8 Versioning and releases

- SemVer, one version for the whole workspace. `Cargo.lock` **is committed** (these are applications).
- Tagged releases are built by CI (Linux musl x86_64/aarch64/armv7, Windows x86_64) with SHA-256 checksums and Ed25519 signatures.

---

## 3. Clean code checklist (reviewers use this)

- [ ] Functions do one thing and are ≤ ~50 lines; files stay ≤ ~500 lines. The web UI is split by route (`hub/src/app/pages/`) and by component (`hub/src/app/components/`); browser APIs go through `app/browser.rs`, API calls through `app/http.rs`.
- [ ] Names say *what*, comments say *why*.
- [ ] No duplicated logic (see §2.3). If you copy-paste, extract it.
- [ ] No magic numbers. Named constants or config (e.g. `POLL_INTERVAL`, `PROBE_TIMEOUT_DEFAULT`).
- [ ] Pure logic is separated from I/O so it can be unit-tested.
- [ ] Public items have `///` docs.
- [ ] No dead code, commented-out code, or leftover template files (e.g. `hub/README.md` is the Leptos starter text).

---

## 4. Testing strategy

| Layer | What | How | Examples |
|---|---|---|---|
| **Unit** | Pure logic | `#[cfg(test)]` modules | severity rules, uptime bucketing, alert/flap state machine, config validation, enrollment-code encode/decode, permission checks |
| **Storage conformance** | Every backend behaves identically | one generic suite (`storage::conformance`) run per backend. SQLite always; others when `BLENTINEL_TEST_<BACKEND>_URL` is set | dashboard aggregation, time bucketing across DST, retention/rollups, scoping, migration from the previous schema |
| **API integration** | Handlers + auth | `tower::ServiceExt::oneshot` against the real `Router` | 401/403 matrix per role, JSON error shape, silence round-trip |
| **Protocol end-to-end** | Probe ↔ hub | spawn hub in-process + probe transport against it | enroll → report → config push → command → buffered reports after outage |
| **Browser (UI)** | The web UI against a real hub and probe | Playwright (`hub/end2end`): its global setup starts a hub (`--init` config in a temp folder) and registers a real probe, as a user would | login/logout and redirects, dashboard severity, 15 s refresh and its cleanup, theme persistence, company drill-down, uptime range, silence round-trip, admin version, the probe running the checks the hub gave it (the setup creates them through the API). Any browser console error or WASM panic fails the test. Page loads wait for `data-hydrated` on `<html>`, which the app sets once the WASM has hydrated the page (`hub_lib::hydrate`); clicking earlier does nothing. |
| **Security regression** | Crypto & authz | unit + integration | tampered signature rejected, revoked probe rejected, replayed report rejected, client viewer cannot see other companies |

Rules:

- **Every bug fix starts with a failing test.**
- Tests are deterministic. Time is injected: `AppState.clock` (`hub_lib::clock::Clock`) in handlers, and a plain `now: DateTime<Utc>` parameter in repository functions. Nothing calls `Utc::now()` directly inside logic under test. (Remaining direct calls: archive engine, e-mail timestamps, auth sessions. They get fixed when those areas are next touched.)
- Coverage is a signal, not a target. Aim for full coverage of `common`, `service`, and alerting logic.
- **Known bugs are pinned by ignored tests.** A test named `known_issue_*` asserts the *intended* behaviour and carries `#[ignore = "known issue D-NN: …"]`, linked to a `DECISIONS.md` entry. Fixing the bug means removing the `#[ignore]` in the same change.

### 4.1 Running the tests

| Command | What it runs |
|---|---|
| `cargo test-all` | Everything: all crates **including the hub's server-side tests**. Alias for `cargo test --workspace --features hub/ssr` (`.cargo/config.toml`). Plain `cargo test --workspace` silently skips the hub's server code, because the hub has no default features. |
| `cargo test -p hub --features ssr --test ingest` | One hub integration-test file (`hub/tests/ingest.rs`) |
| `cargo test-all -- --ignored` | Only the `known_issue_*` tests (they are expected to fail until fixed) |
| `npm test` in `hub/end2end` | The browser tests (after `npm run build`). Setup and options: CONTRIBUTING.md, "Browser tests". |

### 4.2 The hub test harness (`hub/tests/support/mod.rs`)

- `TestHub::new(&[&probe, …])` builds the **real** `AppState` and `hub_lib::routes::api_router()` on a fresh SQLite file in a temp directory. It loads a real config file through `config::load_from`, so validation runs too. Requests go through the router in-process: no sockets, no browser, no WASM build.
- `hub.clock` is a `ManualClock` (starts at `2026-03-10T12:00:00Z`), so tests use `hub.clock.advance(..)` instead of sleeping.
- `TestProbe` has its own Ed25519 identity and seals and signs reports with **`common::crypto`, the same code the real probe runs**, so protocol tests exercise the real wire format.
- Helpers: `hub.login()`, `hub.get_json(path)`, `hub.report_now(&probe, company, results)`, `hub.sweep_expired()`, `hub.seed_report(..)` (history, bypassing ingest), `ping(..)`, `metric(..)`.
- Checks (D-49): tests write results by check name and spec (`ping(..)`, `metric(..)`); each gets a stable ID, `check_id(probe_id, name)`, and the harness creates the matching check record before a report is sent or seeded (`hub.ensure_checks`), as an admin would. `hub.add_check(&probe, name, spec)` creates one directly; `hub.send_report_for_ack` opens the hub's answer, config included.
- Alerts: `TestHub::with_alerts(..)` enables alerting; `hub.notifier` is a `RecordingNotifier` (the `Notifier` trait, `service/alerts.rs`) that records instead of sending. `hub.wait_for_alerts(n)` waits for alerts sent by background tasks. Prefer calling `service::alerts` directly for deterministic tests.
- API assertions compare **raw JSON** (`serde_json::Value`) because that wire format is the contract the UI depends on.

---

## 5. CI (GitHub Actions): target design, on hold

> **Status: on hold (D-24).** CI configuration is owner-controlled. Until it is enabled, run steps 1–3 locally before every commit. This section describes the intended pipeline only.

On every PR:

1. `cargo fmt --check`
2. `cargo lint` and `cargo lint-wasm`
3. `cargo test-all` on `ubuntu-latest` and `windows-latest`
4. `cargo deny check` (licenses, advisories, duplicate versions)
5. Docs: markdown link check (e.g. `lychee`) so the README never points at missing files
6. Build probe for all release targets (catches FP-2 regressions, e.g. an accidental OpenSSL link)

On tag `v*`: the release workflow builds, signs, checksums, and publishes the binaries and Docker image, and attaches the CHANGELOG section.

---

## 6. Documentation standards

- Layout (owned and maintained per PP-4):

```
README.md                     What it is, why, getting started, links. Short (< ~100 lines).
CONTRIBUTING.md               Build, test, publish, manual, how changes are made
CHANGELOG.md                  Every user-visible change, incl. Breaking
docs/PRINCIPLES.md  docs/ENGINEERING.md  docs/ROADMAP.md  docs/DECISIONS.md
docs/architecture.md          Components, layering, report flow, severity, background tasks
docs/security.md              Signatures, sealing, replay protection, TLS, files to protect
docs/install/                 hub.md, probe.md, https.md
docs/guide/                   users.md, companies-sites.md, audit.md, alerting.md, data-retention.md, troubleshooting.md
docs/reference/               configuration.md, checks.md, cli.md, api.md, protocol.md, database.md
docs/proposals/NNNN-*.md      Design proposals (status tracked in ROADMAP)
docs/manual/                  chapters.txt, manual.yaml, header.tex: the generated PDF manual (D-09)
```

- A new user-facing page goes into `docs/manual/chapters.txt` too. Keep diagrams in plain ASCII so they render in the PDF.
- Check links before committing: every relative link in `README.md`, `CONTRIBUTING.md` and `docs/` must resolve (a link check joins CI when it is enabled).

- User-facing docs describe **released** behaviour. Unreleased behaviour lives in proposals.
- One source per fact. Link instead of copying.
