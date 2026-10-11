# Blentinel — Roadmap & Backlog

> **The single source of truth for what's planned.** It replaces `TODO.md` and `PROGRESS.md`.
> Bound by [`PRINCIPLES.md`](PRINCIPLES.md) · built to [`ENGINEERING.md`](ENGINEERING.md) · challenges and decisions in [`DECISIONS.md`](DECISIONS.md) · designs in [`proposals/`](proposals/).
>
> Update the **Status** column in the same PR that changes it.

_Last reviewed: 2026-09-30_

## Where we are (2026-10-11)

**Done:**
- **M0 · Foundations:** everything except M0-03 and M0-13 (CI and release workflow, on hold: D-24).
- **M1 · Accounts & Inventory:** M1-01 to M1-06 (users, roles and setup; companies and sites; company scoping; HTTPS by default; audit log).
- **M2 · Zero-touch onboarding:**
  - M2-01: enrollment codes, probe records, pinned HTTPS, the hub assigns the tenant.
  - M2-03: the hub's signed, sealed answer to each report.
  - M2-04: checks as hub records, sent to probes in that answer and cached on disk; the probe has no config file (D-49).
  - M2-05: the probe page: checks, results, interval, sync state, copy from another probe (D-50).
  - M2-07: the hub is one binary, its web UI embedded (D-51).
  - M2-08: the hub installs, checks, backs up and removes itself (D-52).
  - M2-09: `probe install`, the hub serves probes, Add probe gives a checked one-line command (D-53).
  - Enrollment codes without a use limit by default; the company page shows details, Edit opens the form (D-54).
  - M2-15: signed releases, probes update themselves from their hub, with self-test, watchdog and rollback (D-55).
  - M2-06: the command channel; "Run checks now" on the probe page (D-56).
  - M2-10: the one-line hub install, `hub.sh`, from GitHub Releases, signature and checksums checked; a static musl hub (D-57).
  - M2-12: daily backups set on the admin page; a backup before every database upgrade, put back if it fails (D-60).
  - M2-13: the hub rotates its certificate and key over the protocol; probes pin the certificate's public key (D-61).
  - M2-14: the certificate from Let's Encrypt (or another ACME authority), issued for the hub's own key so probes notice nothing, renewed by itself (D-62).
  - M2-02 was dropped.
- **M3 · Trustworthy alerting:**
  - M3-02: flap suppression: alerts after 2 results in a row by default, set per check (D-63).
  - M3-01: Up/Warning/Down/Unknown judged on the hub, thresholds by check, site, company and hub, set in the UI (D-64).
  - M3-04: maintenance windows (once, weekly, monthly, in a site's time zone) and silences of a check, probe, site or company (D-65).
  - M3-03: alert channels in the UI (e-mail, signed webhook, Teams, Slack, Discord, Telegram), routing rules, Send test, secrets sealed with `hub_data.key`; `[alerts]` removed (D-66).
  - M3-05: incidents (one per outage, of a check or a probe), with a timeline, acknowledgement and notes; alerts link to them (D-67).
  - M3-07: check dependencies: behind a check that is down, a check is Unknown and quiet; the root cause's alert names it (D-68).
  - M3-06: reminders and escalation per routing rule until someone acknowledges, kept with the incident (D-69).
  - M3-08: ntfy channels on ntfy.sh or a self-hosted server, with a self-hosting guide; Pushover declined (D-70).
  - M3-09: the daily digest e-mail, also on a quiet day (D-71).
  - M3-10 and M3-11 deferred (PSA connectors, SMS: paid, closed services); instead alerts carry a stable incident tag for PSA tickets by e-mail or webhook (D-72).
- **M4 · Fleet operations:**
  - M4-02: each report carries the probe's health (uptime, reports held back, clock, last problem; protocol v8); the Probes page shows every probe and flags those that need attention (D-73).
  - M4-01: managed rollout: canary probes first, then every probe after a soak time; pause, resume, roll out now; each company follows, is off or pinned, with an update window in its sites' local time; signed rollbacks (D-74).
  - M4-04: `hub upgrade` (signed release, self-test on a copy of the database, backup, switch, rollback of program and database) and the admin page's opt-in check for a newer version (D-75).
  - M4-03: commands within seconds: the probe asks for commands between reports, the hub holds the request until one is queued and sets the pace (D-76). **M4 is done.**
- **M5 · Monitoring depth:**
  - M5-01: the `tls` check: days until a certificate expires, Down when expired or not accepted (or dates only), thresholds that fall (warning at 21 days, down at 7) (D-77).
  - M5-02: an `http` check's expectations: status codes, text that must or mustn't be there, GET or HEAD, redirects, headers, any certificate; the probe never asks the machine's certificate store (D-78).
  - M5-03: the `file` check: the newest file of a path (a name pattern, subfolders), its age, size and a smallest size; age thresholds in hours (warning at 26, down at 50) (D-79).

**Next, in order:**
1. **M5 · Monitoring depth**, in its order: **M5-04** (ping: packet loss, jitter, TCP fallback) is next. M2-11 (Docker) is on hold until a release candidate (D-58).

**Also open:**
- M1-07 to M1-10 (API tokens, TOTP, SSO, data purge).
- M0-26 (an intermittent browser test).
- M4-04: run `hub upgrade` once on a systemd host (the ubuntu-test VM): the real service start, health wait and rollback.
- Secrets on a check (sealed, write-only), for HTTP basic auth and authenticated database checks (D-78): no roadmap item yet.
- M5-04 onwards; M3-10, M3-11 (deferred).

**Production note (M2-13):** protocol v7 and a new certificate pin. Probes enrolled over **HTTPS** can't update themselves to this version (they hold the old kind of pin; the update rolls back): reinstall them once with Add probe's command. Probes on plain HTTP update themselves when the release is approved. Make new enrollment codes (old ones are refused).

**Production note (M2-06):** protocol v6. The hub accepts v5 and v6, so probes from M2-15 keep reporting; approve the new release on the admin page and they update themselves to v6 (only then can they take commands). v4 probes (from before M2-15) are now refused: reinstall them once with Add probe's command.

**Production note (M2-15):** run `blentinelmake release keygen` once on the build machine, then build and install the hub with `--with-probes`: from then on probes update themselves when a release is approved on the admin page. Probes installed before M2-15 can't update themselves: reinstall them once with Add probe's command (since M2-06 the hub refuses them).

**Production note (M2-01):** the owner's hub and probe predate M2-01. After upgrading (hub and probe together: protocol v4), delete the hub database, set it up again, add the companies and sites, reinstall each probe with Add probe's command (stop and remove the old service first), then add its checks on its page (or copy them from another probe). The probe has no config file any more: delete `blentinel_probe.toml`.

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
| **M2** | **Zero-touch onboarding**: enrollment, hub-managed checks, one-command installs, **probe self-update** (0012) | PP-1/PP-2 core: stop editing files on servers and client machines | [0002](proposals/0002-inventory-and-enrollment.md), [0003](proposals/0003-central-config-and-command-channel.md), [0004](proposals/0004-installation-packaging-upgrades.md) |
| **M3** | **Trustworthy alerting**: 4-state status, flap suppression, maintenance, channels | False alerts destroy trust. Must come before adding more checks that could alert. | [0005](proposals/0005-alerting-v2.md) |
| **M4** | **Fleet operations**: managed probe rollout, hub upgrades, probe health | Every new check type (M5) needs a probe upgrade. Self-update makes M5 cheap to roll out. | [0004](proposals/0004-installation-packaging-upgrades.md) §5.4 |
| **M5** | **Monitoring depth**: new check types, templates, discovery | "More metrics" | [0006](proposals/0006-check-catalogue.md) |
| **M6** | **Insight & reporting**: rollups, history, SLA reports, live UI | Uses all of the above | [0007](proposals/0007-history-reporting-storage.md) |
| **M7** | **Optional storage backends**: PostgreSQL, then MySQL/MariaDB, SQL Server, Oracle flavour | Embedded SQLite covers the target scale. External DBs are an option, never a requirement (PP-3). | [0011](proposals/0011-storage-backends.md) |

The **mobile app** and the **product website** are separate projects, tracked at the end of this file (D-23).

Items marked ⚡ are **quick wins** that may be pulled forward into an earlier milestone once M0's test harness exists.

---

## M0 · Foundations

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M0-01 | Repo hygiene: commit `Cargo.lock`; git-ignore `*.key`, `*.token`, local configs, `publish/`, LaTeX build output; root `LICENSE`; `CHANGELOG.md`; `CONTRIBUTING.md` | P0 | 5 | S | 20 | **done** (2026-09-27): MIT licence, `.gitignore` for secrets/runtime files, `CONTRIBUTING.md`; `Cargo.lock` committed by owner | D-07, 0008 |
| M0-06 | `protocol_version` in report + response | P0 | 4 | S | 16 | **done** (2026-09-27): protocol v2 | D-18, D-28 |
| M0-03 | GitHub Actions CI: fmt, clippy `-D warnings`, tests (Linux + Windows), `cargo deny`, link check; delete `.woodpecker.yml` | P0 | 5 | M | 10 | **on hold** (D-24) | D-19, D-21 · *old TODO: "Use GitHub Actions"* |
| M0-04 | Test harness (in-process hub `Router`, temp DB, `Clock` trait) + characterisation tests of current behaviour | P0 | 5 | M | 10 | **done** (2026-09-27): harness + characterisation tests (63 passing after M0-16/18; `known_issue_*` tests pin open bugs); run with `cargo test-all` | 0008 §3.4 |
| M0-05 | Versioned migrations (`sqlx::migrate!`) from a fresh baseline, foreign keys on. No adoption of existing DBs (D-30) | P0 | 5 | M | 10 | **done** (2026-09-27): `hub/migrations/sqlite/`, `db::open` | 0008 §3.5 |
| M0-12 | Docs restructure: track the ignored docs, retire `PROGRESS.md`/`tree.txt`, fix README (URL, typo, headings, cross-compile claim), replace `hub/README.md`, migrate content that exists only in `manual.tex` into `docs/` | P0 | 4 | M | 8 | **done** (2026-09-27): README rewritten (597 → 81 lines), `docs/{architecture,security}.md`, `docs/install/`, `docs/guide/`, `docs/reference/` | 0009, D-08, D-09, D-11 |
| M0-08 | One DTO module (ssr + hydrate) + domain rules (severity, status) in one place; fix the two inconsistent severity orderings | P1 | 4 | S | 12 | **done** (2026-09-27): `hub/src/model.rs` | 0008 #4–5 |
| M0-02 | Workspace `[workspace.package]` + `[workspace.dependencies]`, edition 2024 everywhere | P1 | 2 | S | 6 | **done** (2026-09-27) | 0008 #13 |
| M0-07 | Move duplicated probe/hub modules (crypto, identity, TLS, args, hot-reload) into `common` | P1 | 4 | M | 6 | **done** (2026-09-27): `common::{crypto,fs,watch,cli}`; TLS stays separate (D-37) | 0008 #3 |
| M0-10 | Split `app.rs` (1,960 lines) into `pages/` + `components/`; replace its `unwrap()`s on browser APIs (`window()`, `document()`) with one helper (left from M0-11) | P1 | 3 | M | 4.5 | **done** (2026-09-27): `hub/src/app/{pages,components}/` (largest file 403 lines), `browser.rs` + `http.rs` helpers, no `unwrap` in UI code; browser smoke test with a live probe | 0008 #7 |
| M0-11 | `ApiError` JSON errors, `tracing` in the hub, remove non-test `unwrap()` | P1 | 3 | M | 4.5 | **done** (2026-09-27): `hub/src/error.rs`, `hub/src/logging.rs`, error envelope `common::protocol::ErrorBody` (also on `/api/report`; the probe logs it); server code has no `unwrap`/`expect`. UI `unwrap`s → M0-10 | 0008 #10–12 |
| M0-13 | Release workflow: build matrix, static-link check, SHA256SUMS, Ed25519 signatures, GitHub Release | P1 | 4 | M | 6 | **on hold** (D-24) | 0004 §6 |
| M0-09 | Hub layering `api → service → repo` (incremental, per area) | P1 | 4 | L | 4 | **done** (2026-09-27): `storage/` + `service/{ingest,dashboard,alerts,liveness,retention,silences}` | 0008 §3.8 |
| M0-14 | Storage traits (`storage::Storage`), all SQL moved behind the SQLite implementation, shared conformance test suite. Done together with M0-09. | P1 | 4 | M | 6 | **done** (2026-09-27): `hub/src/storage/`, D-32 | 0011 §3, §7, D-22 |
| M0-15 | Generated manual: pandoc template from the current `manual.tex` preamble, `docs/manual/manual.yaml` chapter order, `blentinelmake docs` target; retire the hand-written `.tex` | P2 | 3 | M | 3 | **done** (2026-09-27): `blentinelmake docs`; `manual.tex` retired | D-09, 0009 |
| M0-16 | **Security:** require a signature on every probe report (unsigned reports were accepted) | P0 | 5 | S | 20 | **done** (2026-09-27) | D-25 |
| M0-18 | Reset probe-expiry alert state when the probe reports again (a second outage never alerted) | P1 | 5 | S | 15 | **done** (2026-09-27) | D-27 |
| M0-17 | **Security:** replay protection for reports (timestamp window + per-probe high-water mark) | P1 | 4 | M | 6 | **done** (2026-09-27) | D-25, D-28 |
| M0-20 | Probe offline buffer: store only undelivered reports, drop permanent rejections, in-order delivery, age + size bounds | P0 | 5 | S | 20 | **done** (2026-09-27) | D-29 |
| M0-19 | Archive viewer: keep heartbeats in archives (probes still reporting today vanish from them), compute archive uptime relative to the archive cutoff and honour the range buttons. Archiving stays (D-39), so this is a fix, not a limitation. | P2 | 3 | S | 6 | **done** (2026-09-28): heartbeats kept, uptime anchored at the archive's newest report, `range` honoured, read-only archive company page | D-26, D-39 |
| M0-21 | Fix alerting bugs: metric thresholds never alert; silence not cleared on recovery | P1 | 5 | S | 15 | **done** (2026-09-27) | D-33 |
| M0-22 | Fix the shipped service installers (hub: user/ownership, no runtime files packaged, Windows service mode; probe: `User=` line, `cap_net_raw`, never delete `identity.key` on upgrade) | P1 | 5 | S | 15 | **done** (2026-09-27): templates + tests; verified on Ubuntu 24.04 with systemd (install, upgrade, capability, ping as non-root) | D-34 |
| M0-23 | **Hot reload of every config setting** (owner request, 2026-09-27). Today these need a restart. **Probe:** `hub_url`, `hub_public_key` (the transport and hub key are created once at startup). **Hub:** `host`, `port`, `[server.tls]` (rebind the listener; reload the certificate), `identity_key_path`, `auth_token_path` (reload the key/token), `state_dir`/`db_path` (reopen the database). Settings that move to the database in M2 (D-04/D-05: probes, alerts, checks) are managed in the UI and don't need file reload. Each reloaded setting needs a test (change file → behaviour changes, invalid file → old config kept). | P2 | 3 | M | 3 | **done** (2026-09-27): all-or-nothing reload, listeners rebind, database/key/token swap, probe `HubLink`; 13 hub + 4 probe tests, live check (D-38) | configuration.md, D-04, D-05 |
| M0-24 | Workspace-wide `cargo fmt` and the remaining clippy warnings (`blentinelmake`, `probe/src/storage.rs`), so the M0 exit criterion "fmt + clippy `-D warnings` pass" holds. One mechanical commit, no behaviour change. | P2 | 2 | S | 4 | **done** (2026-09-27): fmt-clean; `cargo lint` / `cargo lint-wasm` pass on Windows and Linux (`.cargo/config.toml`) | M0 exit criteria |
| M0-25 | Browser (end-to-end) tests for the web UI: replace the Leptos template in `hub/end2end` with a Playwright suite against a real hub and probe | P1 | 4 | S | 12 | **done** (2026-09-27): 10 tests; found the unmute bug (CHANGELOG) | ENGINEERING §4, 0008 #15 |
| M0-26 | Intermittent browser test: "silencing a device and unmuting it again" failed once in a full run (2026-09-29, passes alone and in reruns); likely a race with the probe's 5 s report cycle. Make it wait on the state it checks | P2 | 3 | S | 6 | todo | M0-25 |

**Exit criteria:** see [0008 §4](proposals/0008-codebase-health.md#4-exit-criteria-for-m0).

## M1 · Accounts & Inventory

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M1-01 | Users, roles, argon2id, DB-backed sessions, permission extractor on **every** route | P0 | 5 | L | 6.7 | **done** (2026-09-28, with M1-02): roles Owner/Admin/Technician/Viewer, access declared per route and tested for every route × role, Users + My account pages. Limited-scope users wait for M1-04 |  0001 · *old TODO: "user login, management, permissions"* |
| M1-02 | First-run setup wizard (setup code) + `hub user …` CLI; retire the shared token | P0 | 5 | M | 10 | **done** (2026-09-28, with M1-01): `setup.code`, `/setup`, `hub user list/create/reset-password`; token and `--reset-admin-token` removed (D-42) | 0001 §4.4, D-06 |
| M1-03 | Companies & sites as records (+ migration from free-text ids) and their management UI | P0 | 5 | M | 10 | **done** (2026-09-28): registered by reports (*New*), edited and archived in the UI, backfilled by migration 0003; keys stay the probe-sent text; no site timezone yet (D-44) | 0002 §4.1, §4.6–4.7 |
| M1-04 | Company scoping for Technician/Viewer/Client roles (enforced in the repo layer) | P0 | 4 | M | 8 | **done** (2026-09-28): scopes all / only / all except, Client role, Technicians edit their companies, lists filtered in storage, 404 outside the scope, guard test over every scoped route (D-45) | 0001 §4.1 |
| M1-05 | HTTPS by default for the UI (auto self-signed), `Secure` cookies, CSRF header, login rate limiting | P0 | 4 | M | 8 | **done** (2026-09-28): split mode (UI on HTTPS 3443, probes keep HTTP 3000), `Secure` cookies, `X-Blentinel` CSRF header, per-user and per-IP login limits, `behind_proxy` | D-03, D-43 |
| M1-06 | Audit log (+ UI) | P1 | 4 | S | 12 | **done** (2026-09-28): logins, users, companies, silences, data and archive actions with who/when/where; Audit page with filters and CSV export (D-46) | 0001 |
| M1-07 | Per-user API tokens | P1 | 4 | S | 12 | todo | 0001 §4.3 |
| M1-08 | TOTP two-factor | P2 | 3 | M | 3 | todo | 0001 |
| M1-09 | SSO (OIDC) / LDAP | P3 | 3 | L | 1 | todo | 0001 §2 |
| M1-10 | **Data purge:** delete data older than a period (days / weeks / months / years) or before a date, from the live database or from an archive; delete a whole archive. Permission-gated (M1-01), confirmation with a preview of what goes, audit-logged (M1-06). Complements archiving, which moves data rather than deleting it. | P2 | 4 | M | 4 | todo | D-39 (owner request 2026-09-28) |

## M2 · Zero-touch onboarding

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M2-01 | Probe records + enrollment codes + `/api/enroll`; runtime pinning of hub TLS SPKI + key (no more per-hub probe builds); hub assigns the tenant | P0 | 5 | L | 6.7 | **done** (2026-09-28): Add company / Add site / Add probe in the UI, `probe enroll <code>`, probes pinned to the hub's certificate over HTTPS, reports filed under the probe's record; `[[probes]]`, `/api/handshake` and `hub_cert.pem` removed (D-47) | 0002, D-01, D-02, D-17 |
| M2-02 | One-time import of `[[probes]]` TOML + heartbeats → probe records | P0 | 4 | S | 16 | **dropped** (D-30: pre-alpha, re-enroll probes) | 0002 §4.6, D-04 |
| M2-03 | Hub Ed25519 signing key + `ReportAck` response envelope (config, commands, rotation, revoked) | P0 | 5 | M | 10 | **done** (2026-09-29): protocol v3; every stored report is answered with an ack signed by the hub (key derived from its identity, pinned via the enrollment code), sealed for the probe and bound to the report; a signed revocation stops the probe; clock-skew warning. Config, commands and rotation fields come with M2-04, M2-06, M2-13 (D-48) | 0003 §4.2 |
| M2-04 | Checks as hub records (`CheckId`), typed `CheckSpec` in `common`, **multi-metric results**, config push + on-disk cache, `[[resources]]` import | P0 | 5 | L | 6.7 | **done** (2026-09-30): protocol v4; checks and the report interval are hub records sent in the signed ack when the probe runs another revision (a content digest); the probe caches them in `config.json`, runs a new config at once, refuses an unusable one whole and reports why; results, alert state and silences keyed by check ID; several metrics per result; the probe has no config file. `[[resources]]` import dropped (D-30); checks are managed by API until M2-05 (D-49) | 0003, 0006 §1.4, D-05, D-15, D-49 |
| M2-05 | Checks management UI (per probe), config sync state, the probe's report interval | P0 | 5 | M | 10 | **done** (2026-09-30): a page per probe with its checks and their latest results, a form per kind (validated in the browser by `common::checks`), edit, disable, delete, the report interval, the sync badge, and "copy checks from" another probe. "Run now" came with the command channel (M2-06); templates and bulk actions are later (D-50) | 0003 §4.5, D-50 |
| M2-07 | Hub single binary with embedded web assets | P0 | 5 | M | 10 | **done** (2026-09-30): `hub/build.rs` embeds the built web UI when `BLENTINEL_EMBED_SITE` is set (a two-step cargo-leptos build: the front end, then the server); served with ETags; publish and installers ship only the binary; dev builds serve from disk. Release targets and Docker are M2-11/CI (D-51) | 0004 §4.1, D-11, D-51 |
| M2-08 | `hub install / uninstall / status / doctor / backup / restore`; on Windows, restrict the data folder's ACL to SYSTEM + Administrators (security.md) | P0 | 5 | M | 10 | **done** (2026-09-30): install is idempotent (upgrades in place), listens on all interfaces, private data folder (Linux `0700`, Windows SYSTEM + Administrators via the Win32 API), systemd unit or a real Windows service; uninstall keeps data unless `--delete-data`; status; doctor with fixes; online backup (`VACUUM INTO`) + keys + TLS + config in a `.tar.gz`, restore with files moved aside. Verified on a systemd host and by the owner on Windows (D-52) | 0004 §4.2, D-52 |
| M2-09 | `probe install --code / uninstall / status / doctor`; static musl builds; hub serves probe binaries; "Add probe" wizard with per-OS one-liners | P0 | 5 | L | 6.7 | **done** (2026-09-30): `probe install` copies itself into one private folder, enrolls, registers the service (systemd over D-Bus as a `DynamicUser` with `CAP_NET_RAW`; Windows service `BlentinelProbe`), upgrades in place; uninstall keeps the identity unless `--delete-data`; status; doctor with the hub's reachability and clock; static musl builds; the hub serves `probes/` with SHA-256; Add probe shows a fail-closed, hash-checked one-liner per platform. Verified on a systemd host; Windows one-liner in PowerShell 5.1 and 7 (D-53) | 0004 §5, D-13, D-53 |
| M2-10 | Install scripts (`hub.sh`, `probe.sh`, `probe.ps1`) with checksum/signature verification | P1 | 4 | S | 12 | **done** (2026-10-04): `curl …/releases/latest/download/hub.sh \| sudo sh` checks the release-key signature of `SHA256SUMS` (openssl) and each file's SHA-256, then runs `hub install`; `blentinelmake release hub` makes the GitHub release (static musl hub, probes, `hub.sh`, signed sums); e-mail on rustls, no OpenSSL. `probe.sh`/`probe.ps1` dropped: Add probe's one-liners (D-53) (D-57) | 0004 §4.3, D-57 |
| M2-11 | Docker image + compose file | P1 | 4 | S | 12 | **on hold** (D-58): packaging for a release candidate; the core and the web UI come first | 0004 §4.1 |
| M2-12 | Automatic pre-migration backup; scheduled backups | P1 | 4 | S | 12 | **done** (2026-10-06): daily backups on by default (03:00 UTC, keep 7), set on the admin page with Back up now, the list and Delete; every backup a `hub backup` file in `[backups] path`, its kind in its name; a backup before any schema upgrade, put back if a migration fails (the hub then doesn't start); Download asks for the password again, then a one-time link (D-60) | 0004 §4.5, D-60 |
| M2-06 | Command channel (queued commands, results in next report) | P1 | 4 | M | 6 | **done** (2026-10-04): protocol v6; commands ride in the signed answer, sent in every answer until the probe reports their result (at least once; the probe skips repeats), expire after 3 intervals (≥ 15 min), cancellable until sent; one kind, `run_checks`, behind "Run checks now" and a Commands section on the probe page; audited (D-56) | 0003 §4.3, D-56 |
| M2-13 | TLS cert / hub key rotation over the protocol | P2 | 3 | M | 3 | **done** (2026-10-06): protocol v7; the TLS pin is the certificate's public key; Hub keys on the admin page starts a rotation of the certificate, the key or both; the next pins ride in signed answers, the hub switches when every probe holds them or on Switch now, flags (doesn't force) an overdue one, keeps the previous key for buffered reports, revokes older codes (D-61) | 0002 §4.5, D-61 |
| M2-14 | ACME / Let's Encrypt | P2 | 3 | M | 3 | done (2026-10-06): set on the admin page, issued for the hub's own key (the pin holds), TLS-ALPN-01 or HTTP-01, renewed with a third of its life left; no DNS-01 | 0004 §4.4, D-62 |
| M2-15 | **Probe self-update from the hub, phase 1 (important):** `blentinelmake release keygen/sign` (private key never on the hub), signed releases uploaded to the hub, hub serves the approved version (none by default), probe download + SHA-256 + `--self-test` + switch (systemd / Windows service / by hand) + watchdog and rollback, local emergency brake, `probe_version` in reports (next protocol version), version view on the admin page | P0 | 5 | L | 6.7 | done (2026-10-04): the offer rides in the signed answer to a report; brake `probe updates off`; no update window yet (M4-01) | [0012](proposals/0012-probe-self-update.md), D-40, D-41, D-55 |

**Milestone demo:** fresh VPS → `curl … | sh` → setup wizard → create company/site → Add probe → paste one command on a Windows PC → probe reports → add an HTTP check in the UI → it appears on the probe with **no file edited anywhere**.

## M3 · Trustworthy alerting

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M3-02 | ⚡ Flap suppression (`fail_after` / `recover_after`, default 2) as a pure, tested state machine | P1 | 5 | S | 15 | done (2026-10-06): per check (no global or company default yet), thresholds too; no "Flapping" state | 0005 §2.2, D-63 |
| M3-01 | 4-state status (Up/Warning/Down/Unknown) + threshold hierarchy evaluated on the hub | P1 | 5 | M | 7.5 | done (2026-10-06): CPU/memory/disk usage and latency; limit by limit; set in the UI; no TLS expiry yet (M5-01) | 0005 §2.1, D-16, D-64 |
| M3-04 | Maintenance windows (one-off, recurring, site timezone: adds `sites.timezone`, D-44) + silences at company/site/probe/check scope | P1 | 5 | M | 7.5 | done (2026-10-07): alerts held and sent after the window, left out of uptime; per company page | 0005 §2.4, D-65 |
| M3-03 | Channels in UI: e-mail (moved from TOML), webhook (HMAC), Teams, Slack, Discord; routing rules; "send test" | P1 | 5 | L | 5 | done (2026-10-07): Telegram too; secrets sealed with `hub_data.key`; `[alerts]` removed, not imported | 0005 §2.6, D-04, D-66 |
| M3-05 | Incidents, acknowledge, timeline | P2 | 4 | M | 4 | done (2026-10-07): one per outage of a check or probe, silenced ones too; notes; Clients don't see notes | 0005 §2.3, D-67 |
| M3-07 | Check dependencies (parent down → children Unknown) | P2 | 4 | M | 4 | done (2026-10-07): same probe, chains, per report; parent's alert names them; no gateway suggestion yet | 0005 §2.5, D-68 |
| M3-06 | Re-notification & escalation | P2 | 3 | S | 6 | done (2026-10-07): per routing rule; one escalation level; in the database; no acknowledging from the alert yet | 0005 §2.3, D-69 |
| M3-08 | ntfy / Pushover (Telegram came with M3-03, D-66) | P2 | 3 | S | 6 | done (2026-10-08): ntfy on any server, optional token, self-hosting guide; Pushover declined (paid, closed) | 0005 §2.6, D-70 |
| M3-09 | Daily digest e-mail | P3 | 2 | S | 2 | done (2026-10-08): one for the hub, local hour and zone, to e-mail channels, also when all is well | 0005 §2.6, D-71 |
| M3-10 | PSA ticketing (HaloPSA, ConnectWise, Autotask, Syncro) | P3 | 4 | L | 1.3 | **deferred** (2026-10-08): paid, closed APIs; tickets by e-mail or webhook with a stable incident tag instead; to revisit | 0005 §2.6, D-72 |
| M3-11 | SMS | P3 | 2 | S | 2 | **deferred** (2026-10-08): needs a paid gateway; ntfy and Telegram reach phones; to revisit | 0005 §2.6, D-72 |

## M4 · Fleet operations

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M4-02 | Probe health telemetry (version, queue depth, clock skew, config state, last error) + fleet view | P1 | 4 | S | 12 | done (2026-10-09): `health` in each signed report (protocol v8); the Probes page with what needs attention; the probe page's Health section | 0003 §4.2, 0004, D-73 |
| M4-01 | Probe self-update, phase 2: approved version and update window per company, pause, signed rollback, canary group then the rest, via the command channel (phase 1 is M2-15) | P1 | 5 | L | 5 | done (2026-10-09): rollout through the signed answer, not a command; the window enforced by the hub in each site's time zone; rollbacks as a signed `rollback.json` | [0012](proposals/0012-probe-self-update.md) §7, D-41, D-74 |
| M4-04 | Hub upgrades, never unattended: opt-in "new version available" notice, admin-triggered `hub upgrade` (download, signature check, database backup, self-test on a copy of the database, switch, rollback of binary and database); a hub release carries its matching signed probes | P2 | 4 | M | 4 | done (2026-10-09): a CLI on the server, not a button (the service can't replace its own program); Linux releases only | [0012](proposals/0012-probe-self-update.md) §8, D-41, D-75; after M2-07, M2-08 |
| M4-03 | Long-poll for near-real-time commands ("run now") | P2 | 3 | M | 3 | done (2026-10-09): a separate command poll whose answer sets the pace; the hub holds it up to 25 s, falling back to polling every 30 s | 0003 §4.3, D-76 |

## M5 · Monitoring depth

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M5-01 | ⚡ TLS certificate expiry check | P1 | 5 | S | 15 | done (2026-10-11): `tls` kind, `days_left`; Down when expired or not accepted (`verify` off: dates only); Mozilla's authorities compiled in; limits that fall, 21/7 days by default | 0006 #1, D-77 |
| M5-02 | ⚡ HTTP expectations (status codes, keyword, max latency, headers, auth) | P1 | 5 | S | 15 | done (2026-10-11): status codes, text present and absent, GET/HEAD, redirects, headers, any certificate; max latency is the latency threshold; no auth until checks have sealed secrets; the probe off the OS certificate store | 0006 §2, D-78 |
| M5-03 | ⚡ File/folder age & size (backup freshness) | P1 | 5 | S | 15 | done (2026-10-11): `file` kind: the newest file of a path, name pattern, subfolders, a smallest size; age limits in hours, 26/50 by default | 0006 #2, D-79 |
| M5-04 | Ping: packet loss, jitter, TCP fallback | P1 | 4 | S | 12 | todo | 0006 §2 |
| M5-05 | Disk auto-discovery (all fixed volumes) | P1 | 4 | S | 12 | todo | 0006 §2 |
| M5-07 | DNS check | P1 | 3 | S | 9 | todo | 0006 #4 |
| M5-06 | Service status (Windows SCM, systemd over D-Bus) | P1 | 5 | M | 7.5 | todo | 0006 #3 |
| M5-12 | Check templates (baselines) + host inventory → suggested checks | P2 | 5 | M | 5 | todo | 0006 §4–5 |
| M5-08 | SNMP v2c/v3 generic | P2 | 5 | L | 3.3 | todo | 0006 #6 · *old PROGRESS: printers, NAS* |
| M5-09 | SNMP presets (printer, IF-MIB, Synology/QNAP, APC) | P2 | 4 | M | 4 | todo | 0006 #7 |
| M5-10 | Database liveness (SQL Server, MySQL, PostgreSQL handshakes) | P2 | 4 | M | 4 | todo | 0006 #8 · *old TODO: "database plugin module"* |
| M5-11 | **Pending updates check** (monitor only, D-59): a check kind reporting pending updates (how many, how many security), reboot required and when the OS last looked; Windows through the Windows Update Agent COM API, Linux through PackageKit over D-Bus where present, else the distribution's own status files (`/var/run/reboot-required`, Ubuntu's `updates-available`). Read-only, no tools run (FP-2); a slow search runs on its own cadence, not every interval | P2 | 4 | M | 4 | todo | 0006 #9, D-59 |
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
| M6-07 | Archive feature reassessment (export vs. remove) | P2 | 2 | S | 4 | dropped (2026-09-28): archiving stays | D-20, D-39 |
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

## To revisit (unscheduled)

Ideas the owner wants kept, but not planned yet. Each one gets a proposal and a milestone when its time comes.

| ID | Item | Pri | Status | Notes |
|---|---|---|---|---|
| R-01 | **Theming / templates for the web UI.** Study how each person or company running a hub could give it their own design (colours, fonts, layout templates) without forking. Options to compare: CSS custom properties + a theme file, selectable templates, a theme upload in the admin UI. Keep the default theme; themes must not be able to break the UI or inject scripts. | P3 | todo (study) | owner request 2026-09-27 |
| R-01a | **Hub operator logo:** the admin uploads their company's logo in the UI (stored by the hub, not a file edited by hand, PP-2), shown in the header and on reports. The header already has a hidden placeholder (`/logo.png`, which 404s today). | P3 | todo | part of R-01 |
| R-01b | **Logo / image per monitored company and site**, shown on the dashboard cards and detail pages (e.g. the client's logo, a photo of the site). Needs company/site records in the database (M1 inventory). | P3 | todo | part of R-01; after M1 |
| R-02 | Pick up a renewed TLS certificate (e.g. from certbot) without a config change: watch `cert_path`/`key_path` and reload them in place | P3 | todo | follow-up of M0-23, D-38 |

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
