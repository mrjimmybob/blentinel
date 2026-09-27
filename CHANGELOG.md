# Changelog

All notable changes to Blentinel are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Breaking (pre-alpha, D-30)
- **Delete the hub database before upgrading.** The schema is now managed by versioned migrations (`hub/migrations/sqlite/`). A database created by an earlier hub is refused at startup with instructions. Stop the hub, delete `blentinel.db` (and `-wal`/`-shm`), and start it again. Probe identities and the `[[probes]]` whitelist are unaffected.
- Upgrade the hub before the probes (protocol v2, see below).
- **New hub install layout** from `install_hub_service.sh`: program in `/opt/blentinel/hub`, config in `/etc/blentinel/blentinel_hub.toml`, data in `/var/lib/blentinel`. On Windows: a scheduled task, with the program in `C:\Program Files\Blentinel\Hub` and data in `C:\ProgramData\Blentinel\Hub`. Existing installs from the old scripts didn't work anyway (D-34); reinstall with the new script.

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
- `cargo lint` and `cargo lint-wasm` aliases (clippy with warnings as errors, for the server and the browser code). The workspace is now `cargo fmt`-clean and warning-free on Windows and Linux (M0-24). `.git-blame-ignore-revs` hides the formatting commit from `git blame`.
- `known_issue_*` tests (ignored) that pin confirmed bugs until they are fixed. Still open: the archive viewer showing almost nothing (D-26).
- Alerting tests (`hub/tests/alerts.rs`) through a `Notifier` interface: SMTP in production, a recording notifier in tests. Alerts were previously untestable.

### Changed
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
