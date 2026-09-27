# Blentinel — Engineering Standards

How we build Blentinel. This complements the Rust style rules in [`rust.instructions.md`](../rust.instructions.md), which remain in force, and is bound by [`PRINCIPLES.md`](PRINCIPLES.md).

---

## 1. Definition of Done

A change is done only when **all** of the following are true:

1. **Principle check passed.** Include the table from `PRINCIPLES.md` in the PR description.
2. **Tests added or updated** for the new or changed behaviour (see §4), and `cargo test --workspace` passes.
3. **Quality gates pass:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, tests on Linux and Windows. Run them locally until CI is enabled; CI is owner-controlled (D-24).
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
storage/       storage::Storage traits + conformance test suite (proposal 0011).
storage-sqlite/  Default embedded backend.   storage-postgres/, … optional backends (feature-gated).
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
- Services depend on `Arc<dyn Storage>`, never on a concrete backend or driver type.
- Rules that show up in several places are written **once** in `common` or `service` and called from everywhere. Examples: the severity rule (currently duplicated in `app.rs` and implied in alerts) and the resource key format (currently built with `format!` in 3+ places).

### 2.3 DRY: one definition of each thing

| Thing | Single source |
|---|---|
| Wire/API DTOs | One module compiled for both `ssr` and `hydrate` (today they are duplicated in `db.rs` and `app.rs`) |
| Probe↔hub protocol | `common::protocol` with an explicit `protocol_version` |
| Validation of checks/config | `common` (used by the hub UI, hub API **and** probe) |
| Crypto, identity, TLS pinning, hot reload, arg parsing | `common` (today: separate copies in `probe/src` and `hub/src`) |
| Default config + its documentation | The generator behind `--init` is the source; docs embed its output |
| Build steps | `blentinelmake`; CI invokes it instead of re-implementing it |
| Dependency versions | `[workspace.dependencies]` in the root `Cargo.toml` |

### 2.4 Types over strings

- Status is an enum (`Up | Warning | Down | Unknown`), not a `format!("{:?}")` string.
- IDs are newtypes (`ProbeId`, `CompanyId`, `CheckId`), so a probe ID can't be passed where a company ID is expected.
- Resource identity is a stable `CheckId`, not the `"company:probe:name:target"` string.

### 2.5 Errors and logging

- Libraries/services: `thiserror` enums. Binaries/glue: `anyhow` with `.context()`.
- The HTTP API returns **JSON errors** `{ "error": { "code": "...", "message": "..." } }` via one `ApiError: IntoResponse`.
- **`tracing` everywhere.** No `println!`/`eprintln!` outside CLI output. Structured fields (`probe_id`, `company_id`), not string concatenation.
- Never log secrets, tokens, password hashes, or decrypted payloads, except under the explicit `--debug` flag, and redacted even then.

### 2.6 Database

- **Embedded SQLite by default; external backends optional** (PP-3, D-22, proposal 0011).
- **Versioned migrations** (`sqlx::migrate!`) replace `CREATE TABLE IF NOT EXISTS` in `setup_tables`. Each backend has its own migration folder with the **same version numbers**, enforced by a schema-parity test.
- SQLite: `PRAGMA foreign_keys = ON`. Every table has an explicit primary key and the indexes that queries use.
- Automatic **pre-migration backup** on hub upgrade (SQLite: file snapshot; external: logical export).
- A new backend is "supported" only when it passes the **shared conformance suite**.

### 2.7 Protocol compatibility

- Reports carry `protocol_version`. The hub accepts the **current and previous** version. The probe refuses to downgrade.
- Breaking protocol changes require a `DECISIONS.md` entry and a release note.

### 2.8 Versioning and releases

- SemVer, one version for the whole workspace. `Cargo.lock` **is committed** (these are applications).
- Tagged releases are built by CI (Linux musl x86_64/aarch64/armv7, Windows x86_64) with SHA-256 checksums and Ed25519 signatures.

---

## 3. Clean code checklist (reviewers use this)

- [ ] Functions do one thing and are ≤ ~50 lines; files stay ≤ ~500 lines. (`app.rs` is 1,960 lines today and must be split into `pages/` and `components/`.)
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
| **UI smoke** | Critical flows | Playwright (`hub/end2end`) | login, dashboard renders, drill-down, silence |
| **Security regression** | Crypto & authz | unit + integration | tampered signature rejected, revoked probe rejected, replayed report rejected, client viewer cannot see other companies |

Rules:

- **Every bug fix starts with a failing test.**
- Tests are deterministic. Time is injected (a `Clock` trait); nothing calls `Utc::now()` directly inside logic under test.
- Coverage is a signal, not a target. Aim for full coverage of `common`, `service`, and alerting logic.
- Current state: **1 test in the whole workspace**. Building the test harness is Milestone 0 in the roadmap.

---

## 5. CI (GitHub Actions): target design, on hold

> **Status: on hold (D-24).** CI configuration is owner-controlled. Until it is enabled, run steps 1–3 locally before every commit. This section describes the intended pipeline only.

On every PR:

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace` on `ubuntu-latest` and `windows-latest`
4. `cargo deny check` (licenses, advisories, duplicate versions)
5. Docs: markdown link check (e.g. `lychee`) so the README never points at missing files
6. Build probe for all release targets (catches FP-2 regressions, e.g. an accidental OpenSSL link)

On tag `v*`: the release workflow builds, signs, checksums, and publishes the binaries and Docker image, and attaches the CHANGELOG section.

---

## 6. Documentation standards

- Target layout, owned and maintained per PP-4:

```
README.md                     What it is, why, 5-minute quick start, links. Short.
CHANGELOG.md
docs/PRINCIPLES.md  docs/ENGINEERING.md  docs/ROADMAP.md  docs/DECISIONS.md
docs/install/hub.md           Binary, Docker, service, TLS, backups, upgrades
docs/install/probe.md         One-liner install, manual install, uninstall, troubleshooting
docs/guide/                   Admin & technician guide (companies, sites, checks, alerts, users)
docs/reference/checks.md      Every check type, parameters, examples
docs/reference/config.md      Hub bootstrap config (generated from --init output)
docs/reference/api.md         REST API
docs/architecture.md          Components, protocol, data model
docs/security.md              Threat model, crypto, pinning, key rotation
docs/proposals/NNNN-*.md      Design proposals (status tracked in ROADMAP)
docs/manual/                  pandoc template + chapter order (manual.yaml) for the generated PDF manual (D-09)
```

- User-facing docs describe **released** behaviour. Unreleased behaviour lives in proposals.
- One source per fact. Link instead of copying.
