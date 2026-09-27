# Blentinel — Roadmap & Backlog

> **The single source of truth for what's planned.** It replaces `TODO.md` and `PROGRESS.md`.
> Bound by [`PRINCIPLES.md`](PRINCIPLES.md) · built to [`ENGINEERING.md`](ENGINEERING.md) · challenges and decisions in [`DECISIONS.md`](DECISIONS.md) · designs in [`proposals/`](proposals/).
>
> Update the **Status** column in the same PR that changes it.

_Last reviewed: 2026-09-27_

---

## How items are weighted

| Field | Scale |
|---|---|
| **Priority** | **P0** blocker: the product isn't credible or safe without it · **P1** high · **P2** medium · **P3** nice to have |
| **Value** | 1–5: impact for an MSP running Blentinel day to day |
| **Effort** | **S** ≤ 2 days · **M** ≤ 1 week · **L** ≤ 3 weeks · **XL** > 3 weeks |
| **Score** | `Value × PriorityFactor ÷ EffortPoints`, where PriorityFactor P0=4, P1=3, P2=2, P3=1 and EffortPoints S=1, M=2, L=3, XL=5. Milestone order comes first because of dependencies. Within a milestone, rows are listed in suggested order: by score, adjusted where one item depends on another (e.g. M1-01 users before M1-04 scoping). |

**Status:** `todo` · `design` (proposal being refined) · `wip` · `done` · `dropped`

---

## Milestones (in delivery order)

| # | Milestone | Why this position | Proposal(s) |
|---|---|---|---|
| **M0** | **Foundations**: tests, migrations, storage traits, DRY cleanup, docs structure (CI on hold, D-24) | Every later milestone changes the schema, protocol, and auth. That's unsafe without tests and migrations. | [0008](proposals/0008-codebase-health.md), [0009](proposals/0009-documentation.md), [0011](proposals/0011-storage-backends.md) §8.1 |
| **M1** | **Accounts & Inventory**: users, roles, companies and sites as records, HTTPS by default | Explicit owner requirement. Client scoping needs company records. | [0001](proposals/0001-users-and-access.md), [0002](proposals/0002-inventory-and-enrollment.md) §4.1 |
| **M2** | **Zero-touch onboarding**: enrollment, hub-managed checks, one-command installs | PP-1/PP-2 core: stop editing files on servers and client machines | [0002](proposals/0002-inventory-and-enrollment.md), [0003](proposals/0003-central-config-and-command-channel.md), [0004](proposals/0004-installation-packaging-upgrades.md) |
| **M3** | **Trustworthy alerting**: 4-state status, flap suppression, maintenance, channels | False alerts destroy trust. Must come before adding more checks that could alert. | [0005](proposals/0005-alerting-v2.md) |
| **M4** | **Fleet operations**: probe self-update, probe health | Every new check type (M5) needs a probe upgrade. Self-update makes M5 cheap to roll out. | [0004](proposals/0004-installation-packaging-upgrades.md) §5.4 |
| **M5** | **Monitoring depth**: new check types, templates, discovery | "More metrics" | [0006](proposals/0006-check-catalogue.md) |
| **M6** | **Insight & reporting**: rollups, history, SLA reports, live UI | Uses all of the above | [0007](proposals/0007-history-reporting-storage.md) |
| **M7** | **Optional storage backends**: PostgreSQL, then MySQL/MariaDB, SQL Server, Oracle flavour | Embedded SQLite covers the target scale. External DBs are an option, never a requirement (PP-3). | [0011](proposals/0011-storage-backends.md) |

The **mobile app** and the **product website** are separate projects, tracked at the end of this file (D-23).

Items marked ⚡ are **quick wins** that may be pulled forward into an earlier milestone once M0's test harness exists.

---

## M0 · Foundations

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M0-01 | Repo hygiene: commit `Cargo.lock`; git-ignore `*.key`, `*.token`, local configs, `publish/`, LaTeX build output; root `LICENSE`; `CHANGELOG.md`; `CONTRIBUTING.md` | P0 | 5 | S | 20 | wip: owner commits `Cargo.lock` | D-07, 0008 |
| M0-06 | `protocol_version` in report + response | P0 | 4 | S | 16 | todo | D-18 |
| M0-03 | GitHub Actions CI: fmt, clippy `-D warnings`, tests (Linux + Windows), `cargo deny`, link check; delete `.woodpecker.yml` | P0 | 5 | M | 10 | **on hold** (D-24) | D-19, D-21 · *old TODO: "Use GitHub Actions"* |
| M0-04 | Test harness (in-process hub `Router`, temp DB, `Clock` trait) + characterisation tests of current behaviour | P0 | 5 | M | 10 | todo | 0008 §3.4 |
| M0-05 | Versioned migrations (`sqlx::migrate!`, baseline = current schema), foreign keys on | P0 | 5 | M | 10 | todo | 0008 §3.5 |
| M0-12 | Docs restructure: track the ignored docs, retire `PROGRESS.md`/`tree.txt`, fix README (URL, typo, headings, cross-compile claim), replace `hub/README.md`, migrate content that exists only in `manual.tex` into `docs/` | P0 | 4 | M | 8 | todo | 0009, D-08, D-09, D-11 |
| M0-08 | One DTO module (ssr + hydrate) + domain rules (severity, status) in one place; fix the two inconsistent severity orderings | P1 | 4 | S | 12 | todo | 0008 #4–5 |
| M0-02 | Workspace `[workspace.package]` + `[workspace.dependencies]`, edition 2024 everywhere | P1 | 2 | S | 6 | todo | 0008 #13 |
| M0-07 | Move duplicated probe/hub modules (crypto, identity, TLS, args, hot-reload) into `common` | P1 | 4 | M | 6 | todo | 0008 #3 |
| M0-10 | Split `app.rs` (1,960 lines) into `pages/` + `components/` | P1 | 3 | M | 4.5 | todo | 0008 #7 |
| M0-11 | `ApiError` JSON errors, `tracing` in the hub, remove non-test `unwrap()` | P1 | 3 | M | 4.5 | todo | 0008 #10–12 |
| M0-13 | Release workflow: build matrix, static-link check, SHA256SUMS, Ed25519 signatures, GitHub Release | P1 | 4 | M | 6 | **on hold** (D-24) | 0004 §6 |
| M0-09 | Hub layering `api → service → repo` (incremental, per area) | P1 | 4 | L | 4 | todo | 0008 §3.8 |
| M0-14 | Storage traits (`storage::Storage`), all SQL moved behind the SQLite implementation, shared conformance test suite. Done together with M0-09. | P1 | 4 | M | 6 | todo | 0011 §3, §7, D-22 |
| M0-15 | Generated manual: pandoc template from the current `manual.tex` preamble, `docs/manual/manual.yaml` chapter order, `blentinelmake docs` target; retire the hand-written `.tex` | P2 | 3 | M | 3 | todo | D-09, 0009 |

**Exit criteria:** see [0008 §4](proposals/0008-codebase-health.md#4-exit-criteria-for-m0).

## M1 · Accounts & Inventory

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M1-01 | Users, roles, argon2id, DB-backed sessions, permission extractor on **every** route | P0 | 5 | L | 6.7 | todo | 0001 · *old TODO: "user login, management, permissions"* |
| M1-02 | First-run setup wizard (setup code) + `hub user …` CLI; retire the shared token | P0 | 5 | M | 10 | todo | 0001 §4.4, D-06 |
| M1-03 | Companies & sites as records (+ migration from free-text ids) and their management UI | P0 | 5 | M | 10 | todo | 0002 §4.1, §4.6–4.7 |
| M1-04 | Company scoping for Technician/Viewer/Client roles (enforced in the repo layer) | P0 | 4 | M | 8 | todo | 0001 §4.1 |
| M1-05 | HTTPS by default for the UI (auto self-signed), `Secure` cookies, CSRF header, login rate limiting | P0 | 4 | M | 8 | todo | D-03 |
| M1-06 | Audit log (+ UI) | P1 | 4 | S | 12 | todo | 0001 |
| M1-07 | Per-user API tokens | P1 | 4 | S | 12 | todo | 0001 §4.3 |
| M1-08 | TOTP two-factor | P2 | 3 | M | 3 | todo | 0001 |
| M1-09 | SSO (OIDC) / LDAP | P3 | 3 | L | 1 | todo | 0001 §2 |

## M2 · Zero-touch onboarding

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M2-01 | Probe records + enrollment codes + `/api/enroll`; runtime pinning of hub TLS SPKI + key (no more per-hub probe builds); hub assigns the tenant | P0 | 5 | L | 6.7 | todo | 0002, D-01, D-02, D-17 |
| M2-02 | One-time import of `[[probes]]` TOML + heartbeats → probe records | P0 | 4 | S | 16 | todo | 0002 §4.6, D-04 |
| M2-03 | Hub Ed25519 signing key + `ReportAck` response envelope (config, commands, rotation, revoked) | P0 | 5 | M | 10 | todo | 0003 §4.2 |
| M2-04 | Checks as hub records (`CheckId`), typed `CheckSpec` in `common`, **multi-metric results**, config push + on-disk cache, `[[resources]]` import | P0 | 5 | L | 6.7 | todo | 0003, 0006 §1.4, D-05, D-15 |
| M2-05 | Checks management UI (per probe), config sync state | P0 | 5 | M | 10 | todo | 0003 §4.5 |
| M2-07 | Hub single binary with embedded web assets | P0 | 5 | M | 10 | todo | 0004 §4.1, D-11 |
| M2-08 | `hub install / uninstall / status / doctor / backup / restore` | P0 | 5 | M | 10 | todo | 0004 §4.2 |
| M2-09 | `probe install --code / uninstall / status / doctor`; static musl builds; hub serves probe binaries; "Add probe" wizard with per-OS one-liners | P0 | 5 | L | 6.7 | todo | 0004 §5, D-13 |
| M2-10 | Install scripts (`hub.sh`, `probe.sh`, `probe.ps1`) with checksum/signature verification | P1 | 4 | S | 12 | todo | 0004 |
| M2-11 | Docker image + compose file | P1 | 4 | S | 12 | todo | 0004 §4.1 |
| M2-12 | Automatic pre-migration backup; scheduled backups | P1 | 4 | S | 12 | todo | 0004 §4.5 |
| M2-06 | Command channel (queued commands, results in next report) | P1 | 4 | M | 6 | todo | 0003 §4.3 |
| M2-13 | TLS cert / hub key rotation over the protocol | P2 | 3 | M | 3 | todo | 0002 §4.5 |
| M2-14 | ACME / Let's Encrypt | P2 | 3 | M | 3 | todo | 0004 §4.4 |

**Milestone demo:** fresh VPS → `curl … | sh` → setup wizard → create company/site → Add probe → paste one command on a Windows PC → probe reports → add an HTTP check in the UI → it appears on the probe with **no file edited anywhere**.

## M3 · Trustworthy alerting

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M3-02 | ⚡ Flap suppression (`fail_after` / `recover_after`, default 2) as a pure, tested state machine | P1 | 5 | S | 15 | todo | 0005 §2.2 |
| M3-01 | 4-state status (Up/Warning/Down/Unknown) + threshold hierarchy evaluated on the hub | P1 | 5 | M | 7.5 | todo | 0005 §2.1, D-16 |
| M3-04 | Maintenance windows (one-off, recurring, site timezone) + silences at company/site/probe/check scope | P1 | 5 | M | 7.5 | todo | 0005 §2.4 |
| M3-03 | Channels in UI: e-mail (moved from TOML), webhook (HMAC), Teams, Slack, Discord; routing rules; "send test" | P1 | 5 | L | 5 | todo | 0005 §2.6, D-04 |
| M3-05 | Incidents, acknowledge, timeline | P2 | 4 | M | 4 | todo | 0005 §2.3 |
| M3-07 | Check dependencies (parent down → children Unknown) | P2 | 4 | M | 4 | todo | 0005 §2.5 |
| M3-06 | Re-notification & escalation | P2 | 3 | S | 6 | todo | 0005 §2.3 |
| M3-08 | ntfy / Pushover / Telegram | P2 | 3 | S | 6 | todo | 0005 §2.6 |
| M3-09 | Daily digest e-mail | P3 | 2 | S | 2 | todo | 0005 §2.6 |
| M3-10 | PSA ticketing (HaloPSA, ConnectWise, Autotask, Syncro) | P3 | 4 | L | 1.3 | todo | 0005 §2.6 |
| M3-11 | SMS | P3 | 2 | S | 2 | todo | 0005 §2.6 |

## M4 · Fleet operations

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M4-02 | Probe health telemetry (version, queue depth, clock skew, config state, last error) + fleet view | P1 | 4 | S | 12 | todo | 0003 §4.2, 0004 |
| M4-01 | Probe self-update: signed releases, download over the pinned channel, swap + restart, automatic rollback, staged/canary rollout | P1 | 5 | L | 5 | todo | 0004 §5.4 |
| M4-03 | Long-poll for near-real-time commands ("run now") | P2 | 3 | M | 3 | todo | 0003 §4.3 |

## M5 · Monitoring depth

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M5-01 | ⚡ TLS certificate expiry check | P1 | 5 | S | 15 | todo | 0006 #1 |
| M5-02 | ⚡ HTTP expectations (status codes, keyword, max latency, headers, auth) | P1 | 5 | S | 15 | todo | 0006 §2 |
| M5-03 | ⚡ File/folder age & size (backup freshness) | P1 | 5 | S | 15 | todo | 0006 #2 |
| M5-04 | Ping: packet loss, jitter, TCP fallback | P1 | 4 | S | 12 | todo | 0006 §2 |
| M5-05 | Disk auto-discovery (all fixed volumes) | P1 | 4 | S | 12 | todo | 0006 §2 |
| M5-07 | DNS check | P1 | 3 | S | 9 | todo | 0006 #4 |
| M5-06 | Service status (Windows SCM, systemd over D-Bus) | P1 | 5 | M | 7.5 | todo | 0006 #3 |
| M5-12 | Check templates (baselines) + host inventory → suggested checks | P2 | 5 | M | 5 | todo | 0006 §4–5 |
| M5-08 | SNMP v2c/v3 generic | P2 | 5 | L | 3.3 | todo | 0006 #6 · *old PROGRESS: printers, NAS* |
| M5-09 | SNMP presets (printer, IF-MIB, Synology/QNAP, APC) | P2 | 4 | M | 4 | todo | 0006 #7 |
| M5-10 | Database liveness (SQL Server, MySQL, PostgreSQL handshakes) | P2 | 4 | M | 4 | todo | 0006 #8 · *old TODO: "database plugin module"* |
| M5-11 | Windows pending reboot / update status | P2 | 4 | M | 4 | todo | 0006 #9 |
| M5-13 | Process check | P2 | 3 | S | 6 | todo | 0006 #5 |
| M5-14 | Windows Event Log check | P2 | 3 | M | 3 | todo | 0006 #10 |
| M5-15 | Network interface metrics | P2 | 2 | S | 4 | todo | 0006 #11 |
| M5-16 | P3 checks: temperature/SMART, mail, NTP, NUT, hypervisors, LDAP | P3 | 3 | XL | 0.6 | todo | 0006 #12–17 |
| M5-17 | Opt-in script check (locally enabled only) | P3 | 3 | M | 1.5 | todo | 0006 #18 |
| M5-18 | Network discovery (probe-side subnet sweep) | P3 | 3 | L | 1 | todo | 0006 §4 |

## M6 · Insight & reporting

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M6-03 | "Problems first" dashboard, site grouping, global search, cert/backup widgets | P1 | 4 | M | 6 | todo | 0007 §3.3 |
| M6-02 | Check detail page (timeline, metric charts, availability, MTTR) | P1 | 5 | M | 7.5 | todo | 0007 §3.1 |
| M6-01 | Raw retention + 5-min/hourly/daily rollups; retention settings UI | P1 | 5 | L | 5 | todo | 0007 §2, D-10 |
| M6-05 | Monthly availability report per company (PDF/CSV, scheduled e-mail, client access) | P1 | 5 | L | 5 | todo | 0007 §3.4 |
| M6-04 | Probe host trends + disk-full forecast | P1 | 4 | M | 6 | todo | 0007 §3.2 |
| M6-06 | Live UI via SSE (replaces the 15 s polling and fixes flicker at the root) | P2 | 4 | M | 4 | todo | 0007 §3.5, D-12 |
| M6-08 | OpenAPI spec for the REST API | P2 | 3 | S | 6 | todo | 0009 §3 |
| M6-07 | Archive feature reassessment (export vs. remove) | P2 | 2 | S | 4 | todo | D-20 |
| M6-09 | Client status page | P3 | 3 | M | 1.5 | todo | 0007 §3.6 |

## M7 · Optional storage backends (future specs)

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M7-01 | PostgreSQL backend (sqlx, pure Rust), selected at runtime via `[database] url` | P2 | 4 | L | 2.7 | todo | 0011 §5 |
| M7-05 | `hub db export / import`: backend-neutral logical dump (also the backup format for external DBs) | P2 | 3 | M | 3 | todo | 0011 §6 |
| M7-07 | Watch libSQL as a drop-in SQLite replacement (replication) | P3 | 2 | S | 2 | todo | 0011 §2 |
| M7-03 | SQL Server backend (`tiberius`, pure Rust) | P3 | 3 | M | 1.5 | todo | 0011 §5 |
| M7-06 | `hub db migrate --to <url>` (e.g. SQLite → PostgreSQL) | P3 | 3 | M | 1.5 | todo | 0011 §6 |
| M7-02 | MySQL / MariaDB backend (sqlx) | P3 | 2 | M | 1 | todo | 0011 §5 |
| M7-04 | Oracle backend: separate build flavour (needs Oracle Instant Client); on request only | P3 | 2 | L | 0.7 | todo | 0011 §5 |

---

## Separate projects (pending, outside the probe–hub core)

These consume the hub's public API and docs. They aren't scheduled in the core milestones (D-23).

| Project | Status | Notes | Ref |
|---|---|---|---|
| **Product website** (marketing / landing page for Blentinel) | pending | Old TODO "Create web page". Suggest its own repo. It can reuse content from `docs/` (principles, features, install quick start) so there's one source of truth. | D-23 |
| **Mobile app** | pending | Spec exists. Refresh it once M1 (API tokens), M3 (4-state status, ack), and M6 (SSE) ship, then prototype (Rork). | [0010](proposals/0010-mobile-app.md), D-14 |

---

## Already done (from the old TODO/PROGRESS)

| Item | Notes |
|---|---|
| Workspace (probe / hub / common) | |
| Probe Ed25519 identity, signing; X25519 + ChaCha20-Poly1305 encryption | PROGRESS said "AES-256". ChaCha20-Poly1305 was chosen, which is equally strong and faster without AES-NI. |
| Probe transport + offline buffer (SQLite queue) | FP-4 |
| Hub probe whitelist, signature verification, expiry "reaper" | Whitelist moves to the DB in M2 |
| Ping / HTTP / TCP / local CPU, memory, disk, uptime checks | |
| Web dashboard (companies → probes → devices), uptime chart | |
| E-mail alerts, per-device silences, thresholds | Superseded by M3 |
| Retention warning + archives + archive viewer | Reassessed in M6 (D-20) |
| Optional HTTPS with pinning (compile-time cert) | Replaced by runtime pinning in M2 (D-01) |
| Config hot reload (probe and hub) | Reduced in scope by D-04/D-05 |
| `blentinelmake` build tool; Windows service support | |
| Migration to GitHub | Done (`d3a2936`) |
