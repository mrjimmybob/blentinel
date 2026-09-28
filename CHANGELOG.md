# Changelog

All notable changes to Blentinel are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Breaking (pre-alpha, D-30)
- **Protocol v3 and new enrollment codes** (M2-03, D-48). Codes now carry the hub's signing key: create new codes after upgrading the hub, and use probes from this version to enroll. The hub accepts protocol versions 2 and 3; v1 reports (without `protocol_version`) are refused.
- **Probes join by enrollment** (M2-01, D-47). The hub's `[[probes]]` whitelist, `/api/handshake`, the probe's compiled-in `hub_cert.pem` (and `probe/build.rs`), and the probe settings `company_id`, `site`, `hub_url` and `hub_public_key` are gone. To upgrade: delete the hub database (migration `0006`), set the hub up again, add the companies and sites under **Companies**, then for each probe **Add probe** and run `probe enroll <code>` on it. `blentinel_probe.toml` keeps only `interval` and `[[resources]]`; the old keys are ignored, so delete them to avoid confusion.
- **The hub decides each probe's company and site;** the `company_id` and `site` in reports are ignored. Reports no longer create companies or sites (D-44's auto-registration): add them in the UI.
- **The web UI moves to HTTPS on port 3443** (M1-05, D-43). HTTPS is on by default, also for configs without `[server.tls]`: the hub creates a self-signed certificate, serves the UI and API on `https://<hub>:3443`, and keeps plain HTTP on `port` (3000) **only for the probe endpoints**, redirecting everything else to HTTPS. Existing probes keep working unchanged. Open port 3443 in the firewall; the browser warns about the self-signed certificate once (compare its fingerprint with the hub log). `https_port` now defaults to 3443, and "HTTPS only on `port`" is now written `https_port = <port>` (it used to be the absence of `https_port`). `enabled = false` still gives plain HTTP for everything.
- **API scripts must send `X-Blentinel: 1` on every POST** (CSRF protection); without it the hub answers `403 csrf`. Probes are unaffected.
- **User accounts replace the admin token** (M1-01, M1-02). `hub_auth.token`, the `auth_token_path` setting and `hub --reset-admin-token` are gone. The database upgrades itself (migration `0002`). After upgrading, start the hub, open the web UI, and create the first Owner with the **setup code** from the hub's log or `<state_dir>/setup.code`. Delete the old `hub_auth.token` file. An old config that still sets `auth_token_path` loads fine; the setting is ignored. Everyone who was logged in must log in again.
- **Delete the hub database before upgrading.** The schema is now managed by versioned migrations (`hub/migrations/sqlite/`). A database created by an earlier hub is refused at startup with instructions. Stop the hub, delete `blentinel.db` (and `-wal`/`-shm`), and start it again. Probe identities and the `[[probes]]` whitelist are unaffected.
- Upgrade the hub before the probes (protocol v2, see below).
- **New hub install layout** from `install_hub_service.sh`: program in `/opt/blentinel/hub`, config in `/etc/blentinel/blentinel_hub.toml`, data in `/var/lib/blentinel`. On Windows: a scheduled task, with the program in `C:\Program Files\Blentinel\Hub` and data in `C:\ProgramData\Blentinel\Hub`. Existing installs from the old scripts didn't work anyway (D-34); reinstall with the new script.

### Added (M2-03: the hub's signed answer, D-48)
- **Every stored report is answered with a `ReportAck`,** signed by the hub and sealed for the probe, and bound to that report. It is the channel that later brings the probe its checks (M2-04) and commands (M2-06).
- **Revocation reaches the probe:** a revoked probe gets a signed answer saying so, stops reporting, clears its buffer, renames `enrollment.json` to `enrollment.revoked.json`, and waits to be enrolled again. A plain `403` (which anyone on the network could send) only drops the report.
- **Clock-skew warning:** the probe warns when its clock is more than a minute off the hub's, before the hub starts refusing its reports.
- The hub's Ed25519 signing key is derived from `hub_identity.key`: nothing new to back up.
- Tests: ack sealing, signature, binding and forward compatibility (`common`); acks for stored and revoked reports (hub API); the probe's handling of trusted, forged, replayed and revoked answers, over real HTTP; a browser test checking that the real probe trusts the real hub's answers.

### Added (M2-01: probe enrollment, D-47)
- **Add probe** on a company's page: choose the site, optionally a name, how many probes may use the code (1) and for how long (24 h), and the hub address probes will use. The hub shows an enrollment code (`BLT1-…`) once. Pending codes are listed and can be revoked.
- **`probe enroll <code>`** joins the hub: the code carries the hub's address, key and certificate fingerprint, so the probe trusts that hub from the first packet and reports over **HTTPS pinned to its certificate**, with the stock binary. Enrolling again moves a probe to another hub, company or site. A probe started before enrolling waits for it.
- **Probe records:** each company page lists its probes (site, state, hostname, version, last seen); rename, move to another site, **Revoke** (`403 probe_revoked`).
- **Add company** (Admins) and **Add site** (also Technicians in their companies).
- API: `POST /api/enroll` (probes), `GET /api/probes`, `POST /api/probes/{id}`, `POST /api/probes/{id}/revoke`, `GET|POST /api/enrollment-codes`, `POST /api/enrollment-codes/{id}/revoke`, `POST /api/companies`, `POST /api/companies/{id}/sites`. Audit actions `probe.enroll`, `probe.update`, `probe.revoke`, `code.create`, `code.revoke`, `company.create`, `site.create` (replacing `company.register`, `site.register`).
- Tests: enrollment API tests (codes expired, used up, revoked, wrong; spoofed company ignored; revoked probes), storage contracts for records and codes, a pinned-TLS test on the probe, and browser tests with a real probe enrolling and reporting over pinned HTTPS.

### Added (M1-06: audit log, D-46)
- **Audit log:** who did what, when and from where: logins and failed logins, logouts, setup, password changes, user management (UI and `hub user`), companies and sites registered by reports or edited, archiving, silences, data deletion, probe removal, archives. Passwords and tokens are never recorded.
- **Audit** page (Owner, Admin): filters by user, action group, target and dates; "Load older"; **Export CSV**. API: `GET /api/audit`, `GET /api/audit.csv`. Guide: `docs/guide/audit.md`.
- The log isn't copied into archives. Writing it is best effort: a failure is logged and never blocks the action.

### Added (M1-04: which companies each user sees, D-45)
- **Company scope per user:** all companies (the default, so a single-admin hub needs nothing), only chosen companies, or all except chosen ones (new companies included). Owners and Admins always see everything. Any number of users can share a company.
- **The Client role** is available: a customer's read-only login for their own companies, without your internal notes and contacts.
- **Technicians can edit the details** (names, contacts, notes, addresses) of the companies in their scope. Archiving a company is now Admin-only.
- **Everything is scoped:** dashboard, companies, probes, devices, uptime, silences, archives. Other companies are left out of lists, and asking for one directly answers `404`, so they can't be discovered.
- **Users page:** a company picker (All / Only these / All except these) when adding or editing a user, and a Companies column. The API's user `all_companies` + `companies` fields are replaced by `scope` (`docs/reference/api.md`).
- Tests: a scoping test for every route open to scoped users (a new data route fails the build until it has one), storage contracts for the three modes, browser tests for a Client and the picker.

### Added (M1-03: companies and sites, D-44)
- **Companies and sites are records.** The first report naming a company or site registers it, marked **New**; nothing to create beforehand. **Companies** in the menu lists them; each company's page edits its display name, contacts and notes, and its sites' names, addresses and notes (Owner, Admin). The dashboard and company pages show the display names. Guide: `docs/guide/companies-sites.md`.
- **Archiving a company** hides it from the dashboard (e.g. a typo in a probe's `company_id`, or a former client); its probes still report and alert, nothing is deleted.
- Existing data is picked up by the migration: every company and site already in the database becomes a record.
- New permission `ManageCompanies` (Owner, Admin); new endpoints `/api/companies…` (`docs/reference/api.md`).
- Known limitation (pre-alpha, D-30): archives created before this version can't be opened in the archive viewer (their database has no `companies` table).

### Added (M1-05: HTTPS by default, D-43)
- **Split mode** (above): passwords and session cookies never cross plain HTTP, probes don't need a rebuild. Session cookies get `Secure` over HTTPS.
- **Login rate limiting:** 5 failed logins per client and username are free, then a lock of 30 s doubling up to 15 min; 20 failures from one client within an hour lock it for 15 min. Wrong setup codes count the same way. Locked attempts get `429 too_many_attempts` with `Retry-After`.
- **CSRF header** on every POST (above), in addition to `SameSite=Strict`.
- **`[server] behind_proxy`** for a hub behind a TLS-terminating reverse proxy (`Secure` cookies, client address from `X-Forwarded-For`).
- Tests: split-mode listeners with a real HTTPS client, CSRF over every POST route, cookies, rate limits; browser tests now run the UI over HTTPS with the probe on HTTP.

### Added (M1-01, M1-02: users and roles)
- **Personal accounts with roles:** Owner, Admin, Technician (views and silences), Viewer (read-only). Log in with username and password; passwords are argon2id-hashed and need at least 12 characters. Guide: `docs/guide/users.md`.
- **First-run setup:** a hub without users creates a one-time setup code (logged, and in `setup.code`), and the web UI's **Set up Blentinel** page creates the first Owner with it.
- **Users page** (Owner, Admin): add users with a temporary password, change role, deactivate, reset password, log a user out. **My account:** change your password, log out your other sessions. The header shows who is logged in, and menus and buttons follow your role.
- **Every API route checks a permission**, declared in one table (`hub/src/routes.rs`); `403 forbidden` otherwise. New endpoints: `/api/auth/login|logout|me`, `/api/setup`, `/api/me/password`, `/api/me/logout-others`, `/api/users…` (`docs/reference/api.md`). `/api/login` and `/api/logout` are replaced.
- **Sessions are stored in the database** (only a hash of the cookie), survive hub restarts, and end after 12 idle hours or 7 days. Deactivating a user or resetting their password ends their sessions. The cookie is now `SameSite=Strict`.
- **`hub user list | create | reset-password`**: account recovery on the server (break-glass), with temporary passwords.
- Archives never contain accounts or sessions.
- Tests: user/session storage contracts, an access test over every route × role, login/session/setup tests, and browser tests for setup, login, a Viewer's restricted view and creating a user.

### Fixed (installers, D-34, M0-22)
- **Upgrading a Linux probe no longer deletes its identity.** The old installer's `rsync --delete` removed `identity.key`, giving the probe a new ID that had to be re-registered. Upgrades now replace only the binary and keep the config.
- The Linux service units start: the probe's `User=` line had an inline comment, and the hub's `--config=…` pointed at a directory in a form the argument parser rejects. The installers now create the `blentinel` service user.
- Ping works without root under systemd (`AmbientCapabilities=CAP_NET_RAW`, which survives upgrades, unlike `setcap`).
- The installers copy only the files a package ships, never runtime files (keys, token, database) left in the package folder.
- The Windows hub installer registers a scheduled task instead of a Windows service the hub can't run as.
- The hub finds its web assets next to its executable, so it can run with any working directory.
- Verified end to end on Ubuntu 24.04 with systemd: install, upgrade (identity, token and config kept; stray runtime files not copied), and ping from the non-root service user.

### Security
- **Private keys and the admin token are now created readable only by their owner** (`0600` on Linux/macOS): `identity.key`, `hub_identity.key`, `hub_auth.token`, and the TLS key. They used to be world-readable (`0644`). The installers' folders already blocked other users, but tighten existing files: `chmod 600 identity.key` (probe), `chmod 600 hub_identity.key hub_auth.token` (hub). On Windows, see `docs/security.md` (D-34 addendum).
- **The hub now rejects unsigned probe reports (403).** Previously the signature was only checked if present, so anyone who could reach the hub could forge reports for any registered probe (a probe's ID and the hub's key are both public). Official probes always sign, so no probe changes are needed (D-25, M0-16).
- **Replay protection.** The hub rejects reports older than the probe's latest stored report (409) and reports outside the accepted time window, more than 5 min in the future or older than 7 days (422). Probe clocks must be within 5 minutes of the hub, so keep NTP running (D-28, M0-17).

### Fixed
- **HTTPS on the hub never started.** The build contains two TLS crypto backends (from different dependencies), so rustls panicked when the hub built its TLS configuration. The hub now picks one (aws-lc-rs). Found by the M0-23 listener tests; now covered by a regression test.
- **Unmuting a device right after muting it didn't unmute it.** The page showed the device as unmuted, but the hub kept the silence, so its alerts stayed suppressed until the silence expired (up to forever). The UI didn't know the new silence's ID and deleted silence 0 instead. It now reloads the silences from the hub after every change (found by the new browser tests, M0-25).
- **The archive viewer showed almost nothing (D-26, M0-19).** Probes that were still reporting disappeared from archives, and the archive uptime chart was always empty (its range ended "now", after all the archived data). Archives now keep every probe they have reports from, the uptime range ends at the archive's newest report, and the `range` parameter is honoured. Archive company cards now open a read-only company page with the probes, devices and uptime chart. A report exactly at the cutoff is no longer copied into the archive as well as kept in the main database. Only new archives benefit; archives made earlier have already lost those probes' records.
- The admin page showed a hand-typed version, build number (`1023`) and build date (`2026-03-08`) that were never updated. It now shows the real version from `Cargo.toml` and no made-up build details (M0-10).
- The dashboard and company pages leaked their 15-second refresh timer's callback on every visit. It is now freed when you leave the page (M0-10).
- The web UI can no longer crash on a missing browser object (no `unwrap()` left in UI code).
- Documentation corrections: payload sealing does **not** give forward secrecy against theft of `hub_identity.key` (earlier docs claimed "perfect forward secrecy"); only resource-level silences exist; the probe's `hub_url` and `hub_public_key` need a restart to change; the hub *can* be cross-compiled.
- `--help` shows the real binary names (`hub`, `probe`), not `blentinel_hub` / `blentinel_probe`.
- A probe that came back online while its "probe offline" e-mail was still being sent could stay marked as offline for alerting, so its next outage wasn't alerted. The alert is now recorded before sending (D-27 addendum).
- README build instructions: `cargo install cargo-leptos` (was `leptos-cli`), the GitHub clone URL (was Codeberg), and the hub *can* be cross-compiled. Build instructions now live only in `CONTRIBUTING.md`.
- **Disk, CPU and memory threshold alerts now fire.** They were only checked when a resource changed between Up and Down, and local metrics are always Up, so a disk at 95 % never alerted. Thresholds now alert once when crossed, stay quiet while exceeded, and re-arm after dropping back (D-33, M0-21).
- **A silence now ends when the silenced resource recovers.** Previously it outlived the outage it was created for. No recovery alert is sent for a silenced outage (D-33, M0-21).
- A probe that went offline a second time no longer stays silent: the "probe offline" alert re-arms whenever the probe reports again (D-27, M0-18).
- **Probe offline buffer no longer grows without limit.** Reports the hub permanently rejects (e.g. while the probe isn't registered yet) are dropped instead of queued forever. Delivered reports are deleted, backlogs are delivered oldest-first, and the buffer is capped at 7 days / 20,000 reports (D-29).
- **Dashboard severity is consistent.** A company with a live outage is now shown red even if another of its probes has expired; previously the card showed amber and hid the outage. Stale "down" counts from an expired probe now make a company *degraded*, not *critical*. The hub computes severity once and returns it as a new `severity` field (`critical` / `degraded` / `healthy`) on dashboard companies and company probes (D-31, M0-08).
- The archive viewer reports probe names the same way as the live API (whitelist name, or empty; the UI shows the short ID).

### Changed (protocol)
- **Probe ↔ hub protocol v2** (M0-06): reports carry a signed `protocol_version`, and responses carry an `X-Blentinel-Protocol` header. The hub still accepts v1 probes unchanged. Upgrade the hub before the probes.
- The hub's `/api/report` status codes are now documented and meaningful to the probe: 200 stored, 409 already have it, 400/401/403/422 permanent rejection, 5xx retry.

### Added
- **Documentation rewrite (M0-12):** a short README, plus `docs/architecture.md`, `docs/security.md`, install guides (`docs/install/hub.md`, `probe.md`, `https.md`), guides (alerting, data retention, troubleshooting) and references (configuration, CLI, HTTP API, probe protocol, database). The content of the old LaTeX manual and the HTTPS/hot-reload notes was moved here and corrected where it had become wrong.
- **Generated PDF manual (M0-15):** `blentinelmake docs` builds `target/manual/blentinel-manual.pdf` from `docs/` with pandoc. The hand-written `manual/manual.tex` is retired (D-09).
- `probe/blentinel_probe.example.toml`: an anonymised example config (documentation IP ranges), kept valid by a unit test that also checks the `probe --init` template. The local `probe/blentinel_probe.toml` is no longer tracked.
- MIT licence (`LICENSE`) for the whole workspace (replaces the Leptos template's Unlicense in `hub/`), and `CONTRIBUTING.md` with build, test and contribution instructions (M0-01).
- Hub integration-test harness (`hub/tests/`): the real API router on a temporary SQLite database with a controllable clock, plus a test probe that seals and signs reports with the production crypto code. Tests cover ingestion, auth, dashboard, uptime, expiry, silences, admin, and alert state. `cargo test-all` runs everything (ROADMAP M0-04).
- **Browser tests for the web UI** (`hub/end2end`, Playwright): a real hub and a registered probe in a temporary folder, driven in Chrome. They cover login, redirects, dashboard, refresh, theme, company drill-down, uptime range, silencing and the admin page. It replaces the Leptos template test. Instructions: CONTRIBUTING.md (M0-25).
- `cargo lint` and `cargo lint-wasm` aliases (clippy with warnings as errors, for the server and the browser code). The workspace is now `cargo fmt`-clean and warning-free on Windows and Linux (M0-24). `.git-blame-ignore-revs` hides the formatting commit from `git blame`.
- `known_issue_*` tests (ignored) that pin confirmed bugs until they are fixed. None are open now.
- Alerting tests (`hub/tests/alerts.rs`) through a `Notifier` interface: SMTP in production, a recording notifier in tests. Alerts were previously untestable.

### Changed
- **Every config setting now reloads without a restart (M0-23, D-38).** On the hub: `host`, `port` and `[server.tls]` (the hub moves its listeners), `state_dir`/`db_path` (switches database; data isn't moved), `identity_key_path` and `auth_token_path`. On the probe: `hub_url` and `hub_public_key`. A reload is all or nothing: if the new file is invalid or something it needs fails (a port in use, a database that can't be opened, a bad hub key), nothing changes and the error is logged. `hub --reset-admin-token` now works on a running hub (the token file is read at each login).
- A probe `hub_url` without `http://` or `https://` is now refused with a clear message.
- **JSON errors everywhere (M0-11, D-35).** Every hub error response is `{"error": {"code", "message"}}`, including `POST /api/report`, instead of plain text or an empty body. Status codes are unchanged. Codes: `reference/api.md`, `reference/protocol.md`. Internal errors say "see the hub log" and no longer include the cause. The admin delete/remove endpoints answer `{"success": true}` instead of `OK`.
- **The probe logs why the hub rejected a report**, e.g. `Hub answered 403 Forbidden: report from unregistered probe a773d201 (unknown_probe)`, instead of just the status.
- **Hub logging uses `tracing`** (timestamp and level on every line) instead of `println!`. Rejected reports, alerts, reloads, archiving and errors are always logged; `--verbose` adds a line per accepted report; `--debug` adds the full report. `RUST_LOG` overrides the flags. Log line texts changed (e.g. `[ALERT ERROR] …` → `ERROR Failed to send … alert`).
- The hub exits with a clear error instead of panicking when its identity key or admin token is unreadable, empty or corrupt. In HTTP+HTTPS mode, a failing HTTP listener now stops the hub instead of dying silently in the background.
- The web UI shows the hub's error message (e.g. why archiving failed), and treats a failed "unmute" as a failure.
- The probe and hub now share key-file handling, the config-file watcher and their common command-line flags (`common::fs`, `common::watch`, `common::cli`) instead of keeping two copies (M0-07, D-37). A probe with an unreadable or corrupt `identity.key` now exits with a clear message instead of panicking. `--config` without a path says `--config requires a value.`
- The web UI code is split into one module per page and per component (`hub/src/app/`), instead of one 1,960-line file (M0-10). No visible change except the fixes below.
- The remaining business rules moved into services (M0-09 complete): `service::alerts` (with a `Notifier` interface and a context struct instead of 11-argument functions), `service::liveness` (probe-expiry sweep, previously in `main.rs`), `service::retention` (storage info and archiving, previously duplicated between the admin API and the background monitor), and `service::silences`. The separate `archive` module is gone. No behaviour change, except `--verbose` expiry messages no longer repeat the timeout.
- Report ingestion is a service (`hub/src/service/ingest.rs`): typed `IngestError`s instead of early HTTP returns, and the authentication steps are a pure function that can be tested without a database. The HTTP handler only maps errors to status codes (same codes as before). `--verbose` now prints one "Report saved" line per accepted report, and `--debug` prints the accepted report rather than every decrypted payload (M0-09).
- The hub reaches its database only through storage traits (`hub/src/storage/`: `ReportStore`, `ProbeStore`, `AlertStore`, `ArchiveStore`, bundled as `Storage`). SQLite is the embedded implementation, and all SQL lives in `storage/sqlite/`. A shared conformance suite (`hub/tests/storage_conformance.rs`) defines the contract for current and future backends. No behaviour change (M0-14, D-32).
- API response types and the rules the UI and server share (severity, resource keys, short probe IDs) are defined once in `hub/src/model.rs` instead of separately in the server and the UI (M0-08).
- All crates share one version and edition (2024) via `[workspace.package]`, and shared dependencies are declared once in `[workspace.dependencies]` (M0-02).
- Database connections now get WAL, foreign keys, and a busy timeout on **every** pooled connection (previously only one), and resource statuses are deleted with their report (`ON DELETE CASCADE`) (M0-05).
- Probe ↔ hub sealing and signatures now live in one place, `common::crypto` (behind the `crypto` feature). The probe and hub no longer carry separate copies. The wire format is unchanged.
- Hub server code moved from `main.rs` into the library crate (`hub_lib`), with the API route table in `routes.rs` and probe endpoints in `ingest.rs`. No behaviour change.
- Time-dependent hub logic reads an injectable clock (`AppState.clock`) instead of the system clock directly.

### Removed
- The single-column `reports(probe_id)` index, now covered by `reports(probe_id, timestamp)`.
- Unused crypto dependencies from the probe (`x25519-dalek`, `chacha20poly1305`) and the hub (`ed25519-dalek`, `chacha20poly1305`).

## [0.1.0]

- Initial versions of the probe, hub, and `blentinelmake`. See git history.
