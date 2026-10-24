# Blentinel — Roadmap & Backlog

> **The single source of truth for what's planned.** It replaces `TODO.md` and `PROGRESS.md`.
> Bound by [`PRINCIPLES.md`](PRINCIPLES.md) · built to [`ENGINEERING.md`](ENGINEERING.md) · challenges and decisions in [`DECISIONS.md`](DECISIONS.md) · designs in [`proposals/`](proposals/).
>
> Update the **Status** column in the same PR that changes it.

_Last reviewed: 2026-09-30_

## Where we are (2026-10-12)

**Release policy (owner, 2026-10-22, D-106):** Blentinel is meant to be released publicly, but **not yet**.
- **Nothing public of any kind:** no alpha, beta or release candidate, no `v*` tag, no GitHub Release.
- **Until then:** builds are internal development builds, deployed to the lab's staging only.
- **Before a first public pre-release can even be discussed, in order:**
  1. the rest of the agreed roadmap;
  2. a redesign of the hub's web UI;
  3. usability testing with real people, and its findings addressed;
  4. release-readiness validation;
  5. the owner's explicit approval.

  That is milestone **M10**. The public release process (proposal 0013's tags, pre-releases and promotion) is switched on only by M10-08.

**What the first release candidate needs (owner, 2026-10-24).** Every outstanding item is a prerequisite unless listed under "Deferred" below, or until the owner decides otherwise.

| | Items | Effort points (S=1, M=2, L=3, XL=5) |
|---|---|---|
| **Pre-RC blockers** (M0–M6, M10-01 to M10-07) | **81 of 109 done (74%)** | **139 of 196 done (71%)** |
| Still open | 28 | 57 |

The open blockers, by milestone:
- **M0** (5): M0-13 phase 4 (C–G); M0-27; M0-28; M0-30; M0-31.
- **M1** (4): M1-07 to M1-10.
- **M5** (4): M5-16h to M5-16k, pending until an account or setting exists on the other side.
- **M6** (8): M6-01 to M6-06, M6-08, M6-09.
- **M10** (7): M10-01 to M10-07. The UI redesign isn't defined yet, so its effort is a placeholder.

**Deferred until after the first RC** (owner, 2026-10-24): not blockers, and not counted above.

| Item | What | Items | Effort points |
|---|---|---|---|
| M2-11 | Docker image and compose file | 1 | 1 |
| M5-19 | The container check | 1 | 2 |
| M7 | Optional storage backends | 7 | 15 |
| M8 | Several hubs on one machine | 6 | 14 |
| **Total** | | **15** | **32** |

**Also outside the count:**
- **The release itself:** M10-08, which switches on the public release process for the first RC.
- **After the first RC:** M9, the public demo (2 items; it waits for M10-07).
- **Deferred earlier, to revisit:** M3-10 and M3-11.
- **Dropped items, and the split M5-16.**

Counted from this file's rows on 2026-10-24. One row can hide much work: M0-13's remaining phase 4, for example, is a single M.

**Done:**
- **M0 · Foundations:** everything except M0-13 (the release workflow: proposal 0013, phases 4 and 6). M0-03, the CI gate, is done and required on `main` (D-103); phase 2 of M0-13, build once, is done (D-104); phase 3, machines as code (M0-29, D-105), is done: both machines rebuilt from fresh VMs with the playbooks.
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
  - M5-06: the `service` check: a Windows service (Service Control Manager) or a systemd unit (D-Bus), running or not; read-only (D-83). On Linux the installed probe now runs as its own user, `blentinel-probe`, so systemd answers it; FP-2 reworded: nothing to install, what every system has may be used (D-84).
  - M5-07: the `dns` check: a name asked of the machine's DNS servers or a named one, record types, the answers that are fine; a ping takes a host name (D-82).
  - M5-05: the `disks` check: every fixed disk the probe finds, judged by the fullest; the alert says which; disks to leave out (D-81).
  - M5-04: a ping sends several echoes: packet loss, slowest answer, jitter; a loss threshold beside the latency (warning at 50 %); a TCP port for hosts that answer no ping; IPv6 and probes without ICMP fixed (D-80).

  - M5-12: the probe says what its machine is made of (protocol v9); the probe page shows it and suggests checks; check templates, built in and saved from a probe (D-85).

  - M5-08: the `snmp` check: values by OID from switches, printers, NAS and UPS, v2c and v3; secrets on a check, sealed and write-only (D-86).
  - M5-09: SNMP tables, shares, rates and limits on a value; templates for printers, switch ports, Synology and QNAP NAS and APC UPS (D-87).
  - M5-10: the `database` check: PostgreSQL, MySQL/MariaDB, SQL Server and MongoDB greeted in their own protocol, without an account (D-88).
  - M5-11: the `updates` check: security updates that wait, a restart that is needed, a system that stopped looking; reported, never installed; a Warning by three limits in days (D-89).
  - M5-13: the `process` check: a program by name and command line, how many run (a fewest and a most), their CPU and memory (D-90).
  - M5-14: the `eventlog` check (Windows): events that are trouble, or that must be there; suggested for disk errors (D-91).
  - M5-15: the `interface` check: a network interface's link, its traffic in and out, errors and speed (D-92).
  - M5-16a: the `smart` check: the drives' own health, read from the machine's system without rights (D-93).
  - M5-16b: the `temperature` check: the hottest of the machine's own sensors, with limits in degrees (D-94).
  - M5-16c: the `mail` check: SMTP, IMAP or POP3 greeted, STARTTLS or TLS started, the certificate judged (D-95).
  - M5-16d: the `ntp` check: a time server asked, the machine's clock offset judged in seconds (D-96).
  - M5-16e: the `nut` check: a UPS's own word through its NUT server: on battery a Warning, battery low Down (D-97).
  - M5-16f: the `hyperv` check: a Hyper-V host's virtual machines, read on the host itself: the named ones must run (D-98).
  - M5-16g: the `ldap` check: a directory server asked for its root entry without a login; a domain controller's state and certificate judged (D-99).
  - M5-17: the `script` check: a script in the probe's own `scripts` folder, off until the machine's administrator makes that folder; the hub sends a file's name only (D-100).
  - M5-18: network discovery: a probe looks at its own network once when asked, where its company allows it; the devices it found suggest checks (protocol v10, D-101).

**Next, in order:**
0. **CI/CD first** (owner, 2026-10-14): [proposal 0013](proposals/0013-ci-cd.md), accepted (D-102, which lifts D-24), in six phases, one at a time; phase 1 (the CI gate, M0-03) is done, phase 2 (build once, M0-13) is done (D-104: CI and Build green on `main` at `9237a5d`); phase 3 (machines as code, M0-29, D-105) is done (2026-10-21): its own staging VM and a Debian 13 `lab-control`, built by the playbooks in `ops/` and rebuilt from fresh VMs by the owner. Phase 4, revised by D-106, is approved (owner, 2026-10-22) and built next, one pull request at a time. It now builds and tests the signed, automated deployment to staging with **internal development builds**: no tag and no public release. The owner turned off the old job on the Debian server that deployed every push to the VPS. M8 (several hubs on one machine) is deferred until after the first RC; M9 (demo) waits for M10-07.
1. **M5 · Monitoring depth**, in its order: M5-16 to M5-18 are done but for what is pending; **M5-19** (containers) is the last of M5 and waits for the owner's word (noted as pending, 2026-10-13). ESXi, Proxmox, Xen and an LDAP login (M5-16h to M5-16k) are pending until the rewrite and its testing are done: they need an account or a setting on the other side (owner, 2026-10-13). M2-11 (Docker) is on hold until a release candidate (D-58).

**Also open:**
- M1-07 to M1-10 (API tokens, TOTP, SSO, data purge).
- M0-26 (an intermittent browser test).
- M4-04: run `hub upgrade` once on a systemd host (the ubuntu-test VM): the real service start, health wait and rollback.
- HTTP basic auth and authenticated database checks, on the secrets a check now has (D-78, D-86): no roadmap item yet.
- `probe install` on a distribution other than Ubuntu (D-84 was verified there only: a Fedora or Debian VM, on request from the owner).
- The `updates` check (D-89) was not run on a Windows machine that has updates waiting or needs a restart (the development machine had none), to do when such a machine is at hand. On Fedora (run on Fedora 44) it counts the updates, and knows neither when the system last looked nor whether a restart is needed.
- M5-19; M3-10, M3-11 (deferred).
- Network discovery (D-101) was run for real on a private Docker network (a gateway and two containers: all three found with their hardware address, name and ports, in 9 s), and its tests on the Ubuntu VM and on Windows 11. Not run: a sweep of a real LAN (the development machine's holds the production server: on the owner's word), from Windows (the neighbour table through WMI), or from an installed probe with the hub showing the result.
- The `script` check (D-100) was run by the probe's tests on the Ubuntu VM and on Windows 11 (real scripts, real refusals), not by an installed probe from its real folder with the hub showing the result: to do with the next install test.
- The `ldap` check (D-99) was run against a Samba domain controller, OpenLDAP and FreeIPA, not against a Windows Server domain controller (Samba answers as one does, by the same attributes), nor against one that isn't synchronised or refuses StartTLS (unit tests with a scripted server).
- The `hyperv` check (D-98) was run on the development machine, which has Hyper-V on and no virtual machine: as an ordinary user (the check says it may not read) and as an administrator (no machine, Up). Not run: on a host with machines; a machine that is off, saved, or in a critical state (those are unit tests on made-up rows). To do on a Hyper-V host with machines.
- The production ESXi host the owner named for tests (ESXi 5.5, 2013) offers no encryption the probe speaks (no forward-secret cipher), and its certificate ran out in August 2025: the probe's `tls` and `http` checks can't reach it either. A future ESXi check needs a newer host to test against (D-98).
- The `smart` check (D-93) was not run against a Linux machine with an NVMe drive, as the installed service on Windows (where it reads temperature and wear), or against a drive that is failing: to do when such a machine is at hand.
- The `temperature` check (D-94) was not run on a Windows machine that has a thermal zone (a laptop), where it reads through `sysinfo`; nor on a Linux machine with a drive among its sensors, with two CPUs, or with thermal zones only (those are unit tests on made-up files): to do when such a machine is at hand.
- The `mail` check (D-95) was run against large public services only: not against a self-hosted server (Postfix, Dovecot, Exchange on site), one with a self-signed certificate, or one that refuses (those are unit tests with scripted servers).
- The `ntp` check (D-96) was run against public time servers only: not against a network's own (a domain controller, a router), a server that refuses or isn't synchronised (unit tests with a scripted server), or a machine whose clock is really off.
- The `nut` check (D-97) was run against a real NUT server with simulated UPSes (the `dummy-ups` driver), not against a real UPS, a Synology or a QNAP: their UPS names and the way they allow an address are from their documentation.

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
| **M7** | **Optional storage backends**: PostgreSQL, then MySQL/MariaDB, SQL Server, Oracle flavour | **Deferred until after the first RC** (owner, 2026-10-24). Embedded SQLite covers the target scale. External DBs are an option, never a requirement (PP-3). | [0011](proposals/0011-storage-backends.md) |
| **M8** | **Several hubs on one machine**: hub instances, one port 443 shared without breaking the pin, a homelab and a work hub on the VPS; later hubs hosted for others | **Deferred until after the first RC** (owner, 2026-10-24). The owner's homelab and work run on one hub today; each should have its own. Paves the way for running hubs for others. After CI/CD (M0-03, M0-13). | none yet |
| **M9** | **Public demo**: a hub with simulated probes, reset every night | Shows Blentinel without showing anyone's network. Pending (owner, 2026-10-14). **Waits for M10-07** (owner, 2026-10-22): a public demo comes only once a release candidate exists (D-106). | none yet |
| **M10** | **Road to the first public release**: the hub's web UI redesigned, tested by real people, their findings addressed, release readiness validated, the owner's approval | Nothing is released publicly before it (D-106). It comes after the rest of the agreed roadmap. Its items are gates: each starts when the one before is done. | none yet (M10-01 writes the UI proposal) |

The **mobile app** and the **product website** are separate projects, tracked at the end of this file (D-23).

Items marked ⚡ are **quick wins** that may be pulled forward into an earlier milestone once M0's test harness exists.

---

## M0 · Foundations

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M0-01 | Repo hygiene: commit `Cargo.lock`; git-ignore `*.key`, `*.token`, local configs, `publish/`, LaTeX build output; root `LICENSE`; `CHANGELOG.md`; `CONTRIBUTING.md` | P0 | 5 | S | 20 | **done** (2026-09-27): MIT licence, `.gitignore` for secrets/runtime files, `CONTRIBUTING.md`; `Cargo.lock` committed by owner | D-07, 0008 |
| M0-06 | `protocol_version` in report + response | P0 | 4 | S | 16 | **done** (2026-09-27): protocol v2 | D-18, D-28 |
| M0-03 | GitHub Actions CI: fmt, clippy `-D warnings`, tests (Linux + Windows), `cargo deny`, link check; delete `.woodpecker.yml` | P0 | 5 | M | 10 | done (2026-10-18): the gate (`blentinelmake ci`, `.github/workflows/ci.yml`, `deny.toml`, the Rust version pinned) is green on GitHub and its six checks are required on `main` by ruleset; its first run on `main` caught a UI race, fixed in PR #2 (D-103). The docs link check moved to M0-27 | 0013 §8, D-102, D-103, D-19, D-21 · *old TODO: "Use GitHub Actions"* |
| M0-04 | Test harness (in-process hub `Router`, temp DB, `Clock` trait) + characterisation tests of current behaviour | P0 | 5 | M | 10 | **done** (2026-09-27): harness + characterisation tests (63 passing after M0-16/18; `known_issue_*` tests pin open bugs); run with `cargo test-all` | 0008 §3.4 |
| M0-05 | Versioned migrations (`sqlx::migrate!`) from a fresh baseline, foreign keys on. No adoption of existing DBs (D-30) | P0 | 5 | M | 10 | **done** (2026-09-27): `hub/migrations/sqlite/`, `db::open` | 0008 §3.5 |
| M0-12 | Docs restructure: track the ignored docs, retire `PROGRESS.md`/`tree.txt`, fix README (URL, typo, headings, cross-compile claim), replace `hub/README.md`, migrate content that exists only in `manual.tex` into `docs/` | P0 | 4 | M | 8 | **done** (2026-09-27): README rewritten (597 → 81 lines), `docs/{architecture,security}.md`, `docs/install/`, `docs/guide/`, `docs/reference/` | 0009, D-08, D-09, D-11 |
| M0-08 | One DTO module (ssr + hydrate) + domain rules (severity, status) in one place; fix the two inconsistent severity orderings | P1 | 4 | S | 12 | **done** (2026-09-27): `hub/src/model.rs` | 0008 #4–5 |
| M0-02 | Workspace `[workspace.package]` + `[workspace.dependencies]`, edition 2024 everywhere | P1 | 2 | S | 6 | **done** (2026-09-27) | 0008 #13 |
| M0-07 | Move duplicated probe/hub modules (crypto, identity, TLS, args, hot-reload) into `common` | P1 | 4 | M | 6 | **done** (2026-09-27): `common::{crypto,fs,watch,cli}`; TLS stays separate (D-37) | 0008 #3 |
| M0-10 | Split `app.rs` (1,960 lines) into `pages/` + `components/`; replace its `unwrap()`s on browser APIs (`window()`, `document()`) with one helper (left from M0-11) | P1 | 3 | M | 4.5 | **done** (2026-09-27): `hub/src/app/{pages,components}/` (largest file 403 lines), `browser.rs` + `http.rs` helpers, no `unwrap` in UI code; browser smoke test with a live probe | 0008 #7 |
| M0-11 | `ApiError` JSON errors, `tracing` in the hub, remove non-test `unwrap()` | P1 | 3 | M | 4.5 | **done** (2026-09-27): `hub/src/error.rs`, `hub/src/logging.rs`, error envelope `common::protocol::ErrorBody` (also on `/api/report`; the probe logs it); server code has no `unwrap`/`expect`. UI `unwrap`s → M0-10 | 0008 #10–12 |
| M0-13 | Release workflow: build matrix, static-link check, SHA256SUMS, Ed25519 signatures, GitHub Release | P1 | 4 | M | 6 | wip: phase 2 done (2026-10-18, build once: `.github/workflows/build.yml`, every target built once, static and processor checks, ARMv6, `release binaries`/`assemble` with signing apart, the install and rollback test on systemd, `commit` in `release.json`; D-104; CI and Build green on `main` at `9237a5d`). Phase 4, revised by D-106, is approved (2026-10-22) and being built (D-107): internal development builds signed for staging and deployed there automatically, with no tag and no GitHub Release. Part A (SemVer pre-release order; staging keys trusted only where a machine opted in) and part B (`hub release approve`) are done; part C (every merge to `main` an internal build `-dev.<N>`, signed with the staging key in the environment `staging` and kept as the artifact `staging-<version>`, with provenance) is in review; D to G to come. Phase 6 (promotion, public releases) is now M10-08, after the owner approves release readiness | 0013, D-102, D-104, 0004 §6 |
| M0-29 | Machines as code (0013 phase 3): `lab-control` and staging built by Ansible playbooks in `ops/`, a private inventory on `lab-control`, the playbooks tested twice in containers on every change; done when the owner rebuilds both from nothing with them | P1 | 4 | M | 8 | done (2026-10-21): PR A (`base`, `firewall`, the `ops` test workflow, `docs/ops.md`, `remote/` deleted), PR B (`bootstrap.sh`, `lab_control`, the test run from `lab-control` as for real) and PR C (`staging`: `hub.sh`'s tools, the hub's ports) merged (#7, #8, #9). The owner rebuilt both machines from their fresh-install snapshots by `docs/ops.md`: `verify.yml` passed, a second apply reported `changed=0`, no bug found | 0013 §10.3, D-102, D-105 |
| M0-27 | Docs link check in CI: every link in `docs/` and the README resolves (moved from M0-03; needs a tool and a rule for external links) | P2 | 2 | S | 2 | todo | 0009 §3, D-103 |
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
| M0-26 | Intermittent browser test: "silencing a device and unmuting it again" failed once in a full run (2026-09-29, passes alone and in reruns); likely a race with the probe's 5 s report cycle. Make it wait on the state it checks | P2 | 3 | S | 6 | **done** (2026-10-22). The cause was not the report cycle but the hub's count of silences: tests run in parallel on one hub, and the maintenance test adds and removes a company silence meanwhile. A temporary test that forced that interleaving failed the old check ("Expected: 2, Received: 1") and passed the new one. The test now watches its own silence, by a reason unique to each attempt. Passed 10 repeats of both files in parallel, and the full suite 3 times | M0-25 |
| M0-28 | Move the probe to zbus 6 (zvariant and zbus_names merged into it; their advisories RUSTSEC-2026-0370 to -0372 are ignored in `deny.toml` until then) | P2 | 2 | S | 2 | todo: once zbus 6 has settled (6.0.0 came out on 2026-10-16) | D-102, D-104 |
| M0-30 | macOS gaps the informational macOS job shows (5 probe tests fail on every run, on `main` too). They belong to a macOS port (0013 §12), not before. **(a)** Four `update::state` tests build their manifest for `Platform::current()`, which is `None` on macOS: there is no macOS probe release. **(b)** The `script` check: Linux refuses to run a file without `#!`, but macOS runs it with `/bin/sh`. So a text file in the scripts folder would run there as a shell script, where the check means to refuse it (`a_file_that_isnt_a_script_says_so`: exit 127, "just: command not found") | P3 | 2 | S | 2 | todo: with a macOS port | 0013 §12, D-100 |
| M0-31 | **Bug, intermittent: a Windows test couldn't get a local port** (WSAENOBUFS). **Test:** `hub/tests/listeners.rs::a_port_that_fails_at_the_switch_falls_back_to_the_old_one`, at `free_port()` (`TcpListener::bind("127.0.0.1:0")`, line 19). **Error:** `Os { code: 10055 }`, "An operation on a socket could not be performed because the system lacked sufficient buffer space or because a queue was full." **Where:** CI run 39875834635, job `test (windows-latest)`, attempt 1 (PR #20, commit `75dcc11`, 2026-10-23); runner image `windows-2025-vs2026` 20261021.278.1, Windows Server 2025. **What the log shows:** it failed 4 ms after the binary started, on its first bind; the binary's 4 other tests, started in parallel in the same process, bound their sockets and passed; about 30 hub test binaries before it passed; no other warning or OS error. PR #20 doesn't touch sockets, and the same test passed on its previous run (`ec35ea3`). **One diagnostic re-run** of the failed job passed (attempt 2). **Possible causes, none confirmed:** (a) a transient Windows error when several threads bind at once at process start; (b) pressure on the runner's ephemeral ports or socket buffers from earlier tests, which the 4 binds that succeeded in the same milliseconds argue against; (c) the runner image or Windows Server 2025's network stack. **Next:** look for other occurrences in CI's history; run the `listeners` binary in a loop on a Windows runner and locally; only then decide on a fix. No retry, and no change to the test, until a cause is shown | P2 | 3 | M | 3 | todo: open, investigating | M0-25, PR #20 |
| M0-32 | **Investigate: do other admin pages lose unsaved input when they reload?** The Alerts page rebuilt its forms on every data reload after an action, which threw away what was typed (CI #69; fixed for Alerts only). The same rebuild-on-reload pattern, a reload counter and a `match` on the loaded data that builds the page's components, is in `components/thresholds.rs`, `pages/maintenance.rs`, `components/backups.rs`, `components/certificate.rs`, `components/hub_keys.rs` and `components/probe_updates.rs`. **For each:** does it hold form input that a reload rebuilds? If so, a browser test that holds the reload and fails, then the Alerts fix (sections built once, forms that keep their edits and follow the hub only when untouched). If not, say why it's safe | P2 | 3 | M | 3 | todo | M0-25, CI #69 |

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
| M2-09 | `probe install --code / uninstall / status / doctor`; static musl builds; hub serves probe binaries; "Add probe" wizard with per-OS one-liners | P0 | 5 | L | 6.7 | **done** (2026-09-30): `probe install` copies itself into one private folder, enrolls, registers the service (systemd over D-Bus with `CAP_NET_RAW`, as a `DynamicUser` until D-84 gave it its own user; Windows service `BlentinelProbe`), upgrades in place; uninstall keeps the identity unless `--delete-data`; status; doctor with the hub's reachability and clock; static musl builds; the hub serves `probes/` with SHA-256; Add probe shows a fail-closed, hash-checked one-liner per platform. Verified on a systemd host; Windows one-liner in PowerShell 5.1 and 7 (D-53) | 0004 §5, D-13, D-53 |
| M2-10 | Install scripts (`hub.sh`, `probe.sh`, `probe.ps1`) with checksum/signature verification | P1 | 4 | S | 12 | **done** (2026-10-04): `curl …/releases/latest/download/hub.sh \| sudo sh` checks the release-key signature of `SHA256SUMS` (openssl) and each file's SHA-256, then runs `hub install`; `blentinelmake release hub` makes the GitHub release (static musl hub, probes, `hub.sh`, signed sums); e-mail on rustls, no OpenSSL. `probe.sh`/`probe.ps1` dropped: Add probe's one-liners (D-53) (D-57) | 0004 §4.3, D-57 |
| M2-11 | Docker image + compose file | P1 | 4 | S | 12 | **deferred until after the first RC** (owner, 2026-10-24; was on hold, D-58): packaging comes after the core and the web UI | 0004 §4.1 |
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
| M5-04 | Ping: packet loss, jitter, TCP fallback | P1 | 4 | S | 12 | done (2026-10-11): 4 echoes by default (1–20); loss, slowest, jitter; a loss threshold from the levels above the check; `tcp_port` when nothing answers; IPv6 and probes without ICMP fixed | 0006 §2, D-80 |
| M5-05 | Disk auto-discovery (all fixed volumes) | P1 | 4 | S | 12 | done (2026-10-11): `disks` kind: every fixed disk found each run, one status (the fullest), the reason names the disk, an ignore list; a check per disk waits for the inventory (M5-12) | 0006 §2, D-81 |
| M5-07 | DNS check | P1 | 3 | S | 9 | done (2026-10-11): `dns` kind (A, AAAA, CNAME, MX, TXT, NS; the machine's servers or a named one; only the expected answers); a ping by host name | 0006 #4, D-82 |
| M5-06 | Service status (Windows SCM, systemd over D-Bus) | P1 | 5 | M | 7.5 | done (2026-10-11): `service` kind, read-only; Windows by service name, Linux by unit; the installed probe runs as its own user so systemd answers it on every distribution (D-84) | 0006 #3, D-83 |
| M5-12 | Check templates (baselines) + host inventory → suggested checks | P2 | 5 | M | 5 | done (2026-10-12): protocol v9, the probe sends its machine's system, disks, running services, interfaces and gateway; the probe page shows them and suggests checks (add one or all); templates: 4 built in (machine, website, backup, domain controller) and saved from a probe's checks. No network discovery, no link from a check back to its template | 0006 §4–5, D-85 |
| M5-08 | SNMP v2c/v3 generic | P2 | 5 | L | 3.3 | done (2026-10-12): `snmp` kind: GET of up to 20 OIDs, a name and an expected value each, numbers as metrics; v2c, and v3 with every authentication and privacy the library has (verified against net-snmp); secrets on a check (sealed, write-only). No walks, no limits on a value: with the presets (M5-09) | 0006 #6, D-86 · *old PROGRESS: printers, NAS* |
| M5-09 | SNMP presets (printer, IF-MIB, Synology/QNAP, APC) | P2 | 4 | M | 4 | done (2026-10-12): five built-in templates that ask for the device and its community; a value can be a table (walk, rows named by a column), a share of another, a rate, scaled, with meanings, and with warning and down limits the hub judges. Not run against a real printer, switch, NAS or UPS yet | 0006 #7, D-87 |
| M5-10 | Database liveness (SQL Server, MySQL, PostgreSQL handshakes) | P2 | 4 | M | 4 | done (2026-10-12): `database` kind with four engines (PostgreSQL, MySQL/MariaDB, SQL Server, MongoDB), one module each, no account; run against real servers of all five products (containers; PostgreSQL 18 and MariaDB on the Ubuntu VM; SQL Server 2022 on a Windows PC). No login and query, no named-instance lookup | 0006 #8, D-88 · *old TODO: "database plugin module"* |
| M5-11 | **Pending updates check** (monitor only, D-59): a check kind reporting pending updates (how many, how many security), reboot required and when the OS last looked; Windows through the Windows Update Agent COM API, Linux through PackageKit over D-Bus where present, else the distribution's own status files (`/var/run/reboot-required`, Ubuntu's `updates-available`). Read-only, no tools run (FP-2); a slow search runs on its own cadence, not every interval | P2 | 4 | M | 4 | done (2026-10-12): `updates` kind: how many updates wait, how many security, a restart needed and since when, when the system last looked; a Warning by three limits in days on the check. Windows through the Update Agent and the registry; Linux from Ubuntu's own count first (it leaves out updates not yet due on the machine, which PackageKit counts), else PackageKit. Read once an hour. Run on Windows 11 (nothing waiting), Ubuntu 26.04 (two security updates waiting; both sources; a rightless user) and Debian 13 (PackageKit) | 0006 #9, D-59 |
| M5-13 | Process check | P2 | 3 | S | 6 | done (2026-10-12): `process` kind: a program by name (wildcards, `.exe` or not) and by text of its command line; Down when fewer than `min` or more than `max` run; their CPU (share of the machine) and memory together. No thresholds. Run on Windows 11 and on Ubuntu as a rightless user, against `ps` | 0006 #5 |
| M5-14 | Windows Event Log check | P2 | 3 | M | 3 | done (2026-10-12): `eventlog` kind: a log, a source, event IDs, a level, the minutes to look back; events are trouble or (`wanted`) must be there; Down or a Warning; the latest event in words. Windows' query API. A "Disk errors" check suggested for Windows probes. Run on Windows 11 against PowerShell's counts | 0006 #10 |
| M5-15 | Network interface metrics | P2 | 2 | S | 4 | done (2026-10-12): `interface` kind: an interface of the probe's machine by name; Down when its link is down or it isn't there; megabits a second in and out since the last report, errors, the link's speed. No thresholds, not suggested. Run on Windows 11 against `Get-NetAdapter` and on Ubuntu against `ip` | 0006 #11, D-92 |
| M5-16 | P3 checks: temperature/SMART, mail, NTP, NUT, hypervisors, LDAP | P3 | 3 | XL | 0.6 | split (2026-10-13) into M5-16a to M5-16g, one check each, in the owner's order (SMART first); their scores are a first guess | 0006 #12–17 |
| M5-16a | Disk health (SMART) | P3 | 4 | M | 4 | done (2026-10-13): `smart` kind: every drive of the probe's machine that reports its health; Down when one says it is failing, a Warning for bad sectors and other signs; temperature, wear and bad sectors reported. Read from udisks2 (Linux) and Windows' storage management, without rights. Run on Windows 11 (eight drives, against `Get-PhysicalDisk`) on Fedora 44 (a SATA SSD, as an ordinary user: healthy, 27 °C, no bad sectors) and on the Ubuntu VM (no drive reports: Down, as meant) | 0006 #12, D-93 |
| M5-16b | Temperature sensors (CPU, board) of the probe's machine | P3 | 2 | S | 4 | done (2026-10-13): `temperature` kind: the hottest of the machine's sensors, or of those named; a new threshold metric in degrees (warning 90 °C built in), and Down at a sensor's own critical limit. Linux: the kernel's sensor list; Windows: the firmware's thermal zone only. Run on Fedora 44 (a laptop: five sensors, as an ordinary user), on the Ubuntu VM and on a Windows 11 desktop (no sensor: Down, as meant) | 0006 #12, D-94 |
| M5-16c | Mail server: SMTP/IMAP/POP3 greeting and STARTTLS | P3 | 3 | M | 3 | done (2026-10-13): `mail` kind: the greeting in SMTP, IMAP or POP3, then STARTTLS, TLS from the start or none; Down without the encryption asked for or with a bad certificate; the certificate's days left judged by the certificate limits. No login, no mail sent. Run against Gmail, Microsoft 365 and GMX (all three protocols, STARTTLS and TLS) from Windows and from the Ubuntu VM | 0006 #13, D-95 |
| M5-16d | NTP offset | P3 | 2 | S | 4 | done (2026-10-13): `ntp` kind: one SNTP question to a time server; the offset of the machine's clock is a new threshold metric in seconds (5 s warning, 60 s Down built in); Down without an answer, on a refusal or an unsynchronised server. Run against five public servers from Windows 11, Fedora 44 and the Ubuntu VM | 0006 #14, D-96 |
| M5-16e | UPS via NUT | P3 | 2 | M | 2 | done (2026-10-13): `nut` kind: the UPS's values read from its NUT server (`upsd`, TCP 3493), by name or the server's only one; Down when the UPS says its battery is low, a Warning on battery and for a battery to replace; charge, runtime and load reported; no login, no command. Run against a real `upsd` 2.8.4 with two simulated UPSes on the Ubuntu VM | 0006 #15, D-97 |
| M5-16f | Hyper-V virtual machines | P3 | 3 | M | 1.5 | done (2026-10-13): `hyperv` kind: the probe on the Hyper-V host reads Hyper-V's own list (WMI, as `Get-VM`), with no account and nothing set up; the machines named must run, a critical state is Down, a lesser failure a Warning; without names every machine is counted. The WMI reader is shared with `smart`. Run on a host without machines only (see Also open) | 0006 #16, D-98 |
| M5-16h | VMware ESXi: VM states over the vSphere API (read-only account), and an ESXi SNMP template | P3 | 3 | L | 1.5 | pending (owner, 2026-10-13): needs an account or SNMP turned on on the host; revisit after the rewrite. No host to test against: the one at hand is ESXi 5.5, whose encryption the probe doesn't speak | 0006 #16, D-98 |
| M5-16i | Proxmox: VM and container states over its API (a read-only API token) | P3 | 3 | M | 1.5 | pending (owner, 2026-10-13): needs a token made on the host; revisit after the rewrite | 0006 #16, D-98 |
| M5-16j | Xen / XCP-ng: VM states over its API (a read-only account) | P3 | 2 | M | 1.5 | pending (owner, 2026-10-13): needs an account on the host; revisit after the rewrite | 0006 #16, D-98 |
| M5-16g | LDAP/AD: is the directory server answering? | P3 | 2 | S | 4 | done (2026-10-13): `ldap` kind: the server's root entry read without a login, plain, with StartTLS or by LDAPS; Down when a domain controller says it isn't synchronised or the server can't serve; the certificate judged as a `tls` check's; limits on the latency. Run against a Samba AD domain controller 4.23 on the Ubuntu VM, OpenLDAP (Debian's) and FreeIPA | 0006 #17, D-99 |
| M5-16k | LDAP: log in with an account (bind) and find an entry | P3 | 2 | S | 2 | pending (owner's rule, 2026-10-13): needs an account made in the directory; revisit after the rewrite | 0006 #17, D-99 |
| M5-17 | Opt-in script check (locally enabled only) | P3 | 3 | M | 1.5 | done (2026-10-13): `script` kind: a file of the probe's `scripts` folder by its name (no path, no arguments), off until that folder is made on the machine; judged as a Nagios plugin (0 Up, 1 Warning, 2 Down), its first line the message, values after a bar kept; links and files others can change refused; stopped after 20 s | 0006 #18, D-100 |
| M5-18 | Network discovery (probe-side subnet sweep) | P3 | 3 | L | 1 | done (2026-10-14): the `discover` command (protocol v10): the probe pings its private networks (at most 1024 addresses), reads the neighbour table, tries 22 well-known ports and asks DNS for names; off per company until an Admin allows it; the probe's page lists the devices and adds the checks they suggest | 0006 §4, D-101 |
| M5-19 | Container check (Docker, Podman): are the named containers running, healthy, not restarting in a loop? (noted by the owner 2026-10-12; priority and score are a first guess, to confirm) | P2 | 4 | M | 4 | deferred until after the first RC (owner, 2026-10-24) | 0006 #19 |

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

**Deferred until after the first release candidate** (owner, 2026-10-24): not an RC blocker.

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M7-01 | PostgreSQL backend (sqlx, pure Rust), selected at runtime via `[database] url` | P2 | 4 | L | 2.7 | todo | 0011 §5 |
| M7-05 | `hub db export / import`: backend-neutral logical dump (also the backup format for external DBs) | P2 | 3 | M | 3 | todo | 0011 §6 |
| M7-07 | Watch libSQL as a drop-in SQLite replacement (replication) | P3 | 2 | S | 2 | todo | 0011 §2 |
| M7-03 | SQL Server backend (`tiberius`, pure Rust) | P3 | 3 | M | 1.5 | todo | 0011 §5 |
| M7-06 | `hub db migrate --to <url>` (e.g. SQLite → PostgreSQL) | P3 | 3 | M | 1.5 | todo | 0011 §6 |
| M7-02 | MySQL / MariaDB backend (sqlx) | P3 | 2 | M | 1 | todo | 0011 §5 |
| M7-04 | Oracle backend: separate build flavour (needs Oracle Instant Client); on request only | P3 | 2 | L | 0.7 | todo | 0011 §5 |

## M8 · Several hubs on one machine

**Deferred until after the first release candidate** (owner, 2026-10-24): not an RC blocker.

The owner's homelab and work share one hub on the VPS today; they should be two hubs, and one machine should be able to run many (owner, 2026-10-14). One hub per tenant (its own user, database and keys) is the isolation the hub already has: this is mostly install and operations work, not a change inside the hub. Comes after CI/CD (M0-03, M0-13), which gives these hubs tested, signed releases to run.

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M8-01 | **Hub instances:** `hub install --instance <name>`: a system user per instance (`blentinel-<name>`), its own data folder, a systemd template unit (`blentinel-hub@<name>`), its own port; one shared binary | P1 | 4 | M | 6 | todo | D-52, D-57 |
| M8-02 | **One port 443 for all of them, the pin kept:** probes pin the hub's own key (D-61), so a reverse proxy that ends TLS breaks every probe. Route by name without decrypting (SNI passthrough: HAProxy, nginx `stream` with `ssl_preread`, caddy-l4); each hub keeps its own certificate, ACME through TLS-ALPN-01 or HTTP-01 forwarded by name (D-62). Fallback: a port per hub (some firewalls block odd ports) | P1 | 4 | M | 6 | todo | D-61, D-62 |
| M8-03 | **Instances upgraded in order** (a canary hub first, as M4-01 does for probes), each with limits (`MemoryMax`, `CPUQuota`) and the sandboxing the probe's service has; `hub upgrade` for all instances | P2 | 3 | M | 3 | todo | D-74, D-75 |
| M8-04 | **Backups off the machine, per instance, and the hubs watched:** a probe on the VPS checking each hub (`http`, `tls`) and a watcher outside the VPS (if the VPS is down, nothing on it can alert) | P1 | 4 | M | 6 | todo | D-60 |
| M8-05 | **Split today's hub:** a homelab hub and a work hub on the VPS; new hubs have new keys, so their probes are enrolled again (pre-alpha, no migration, D-30); the owner says which companies go where | P1 | 4 | S | 12 | todo | |
| M8-06 | **Hubs hosted for others (study):** provisioning, billing, a data processing agreement (GDPR: we would hold their data), abuse, support; a separate project once M8-01 to M8-05 work | P3 | 3 | XL | 0.6 | todo (study) | |

**Caveats to keep in mind:** a shared machine is a shared failure (one VPS down is every hub down, and its watcher must live elsewhere); every instance's keys must stay readable by that instance's user only; a hub hosted for someone else holds their network's data, so a breach of one hub must not reach another (separate users, no shared folders, no shared database); disk and memory are shared, so one busy hub can starve the others without limits.

## M9 · Public demo

**Pending** (owner, 2026-10-14): after M8, and **not before M10-07** (owner, 2026-10-22, D-106): a public demo comes only once a release candidate exists.

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M9-01 | **Probe simulator:** sends signed reports of made-up machines and checks, failing now and then; also useful for load tests. A demo with real probes would show the owner's network to everyone | P3 | 3 | M | 1.5 | pending | |
| M9-02 | **Demo hub:** an M8 instance, a read-only demo login, reset every night, upgraded first with each release | P3 | 3 | S | 3 | pending | |

---

## M10 · Road to the first public release

**The owner's release policy (2026-10-22, D-106).**
- **Not yet:** no public release of any kind, whether alpha, beta or release candidate, and no `v*` tag.
- **Gates, in order:** these items are gates, not features. Each starts when the one before is done, and none is skipped.
- **When M10 starts:** after the rest of the agreed roadmap.
- **After M10:** a first public pre-release can be discussed only after M10-07, and the public release process is switched on only by M10-08.

| ID | Item | Pri | Val | Eff | Score | Status | Ref |
|---|---|---|---|---|---|---|---|
| M10-01 | **Hub web UI redesign: the proposal** (scope to be defined by the owner later). Navigation, layout, information architecture and usability, page by page. **Done when** the owner accepts a proposal that has: an audit of today's UI (every page, every task's path through it, what is hard to find or understand); the target navigation and information architecture; a layout system (structure, spacing, components, light and dark); one list of terms used everywhere; sketches of the main pages; how to get there from today's UI; and which browser tests change | P0 | 5 | M | 10 | todo: to be defined (owner) | D-106 |
| M10-02 | **Hub web UI redesign: built,** as M10-01 says. **Done when** every page in the proposal is built and no page is left in the old layout; the browser tests cover the new pages and pass; the guides and screenshots in `docs/` match the new UI; and the keyboard and contrast checks the proposal names pass | P0 | 5 | XL | 4 | todo: after M10-01 | M10-01 |
| M10-03 | **Usability test plan.** **Done when** the owner accepts a written plan with: who tests (the owner first, then real users, how many and with what background); the tasks, which are installing a hub on a fresh machine, enrolling a probe on Linux and Windows, adding checks and thresholds, setting up alerts, silences and maintenance, users and roles, upgrading, and backup and restore; what is observed (done or not, time, errors, where people got stuck, terms they misread, things they couldn't find); and how findings are recorded and rated (blocker, major, minor) | P0 | 4 | S | 16 | todo: after M10-02 | D-106 |
| M10-04 | **Usability testing with people.** **Done when** the owner's own sessions on a fresh install are done first, then the plan's real users; and every finding is recorded with its task, what happened, and its rating. Installation counts: every session starts from the install instructions, not a prepared hub | P0 | 5 | M | 10 | todo: after M10-03 | M10-03 |
| M10-05 | **Usability findings addressed.** **Done when** every blocker and major finding is fixed and checked again with the task that found it, or the owner accepted it in writing with a reason; and every minor finding is fixed or is a roadmap row | P0 | 5 | L | 6.7 | todo: after M10-04 | M10-04 |
| M10-06 | **Release-readiness validation.** **Done when** a written report shows every check passed:<br>**Platforms:** each Supported platform (0013 §12) installed, enrolled, updated and uninstalled on real hardware or a VM.<br>**Security:** the threat model reviewed; `cargo deny` clean; no secret in the repository or the builds; the signing and deployment path reviewed.<br>**Docs:** every install and upgrade guide followed word for word on a fresh machine.<br>**Install, upgrade and recovery:** an upgrade from the previous internal build; a forced failed upgrade rolled back; backup and restore drilled on real data; probes self-updating through the rollout.<br>**Known issues:** listed.<br>**Licences:** checked | P0 | 5 | L | 6.7 | todo: after M10-05 | 0013, D-106 |
| M10-07 | **The owner's release-readiness approval.** **Done when** the owner has read the M10-06 report and approved release readiness explicitly, in a DECISIONS entry. Only then can a first public pre-release, and what to call it, be discussed | P0 | 5 | S | 20 | todo: after M10-06 | D-106 |
| M10-08 | **Switch on the public release process.** Proposal 0013's public part: `v*` tags, GitHub pre-releases signed for staging, promotion with the production key (phase 6). **Done when** the first public pre-release the owner decided on in M10-07 has gone through it | P1 | 4 | M | 6 | todo: after M10-07 | 0013, D-106 |

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
