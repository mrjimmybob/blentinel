# Changelog

All notable changes to Blentinel are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Breaking (pre-alpha, D-30)
- **Probe downloads come from signed releases** (M2-15, D-55). The hub no longer serves the files in `<data folder>/probes/`: Add probe offers a release the hub holds in `<data folder>/releases/<version>/`, signed with a release key. Run `blentinelmake release keygen` once, then build the hub and probes again (they compile its public key in); `blentinelmake hub publish --with-probes` now needs that key, and `hub install` adds the signed release. A hub built without a key takes no release, so Add probe offers no download.
- **Protocol v5** (M2-15): reports carry the probe's version. The hub accepts v4 and v5, so probes from M2-09 keep reporting to the new hub; they can't update themselves, so install the new probe on them once (Add probe's command).
- **Checks are managed on the hub; the probe has no config file** (M2-04, D-49). `blentinel_probe.toml`, its hot reload and `probe --init` are gone, and `[[resources]]` aren't imported. **Protocol v4:** the hub refuses older probes (`unsupported_protocol_version`), so upgrade the hub and every probe together. Migration `0007` drops earlier results, resource alert state and resource silences. After upgrading: add each probe's checks on its page in the hub (see [reference/checks.md](docs/reference/checks.md)), and delete the old `blentinel_probe.toml`. Check kinds are renamed: `local_cpu` → `cpu`, `local_mem` → `memory`, `local_disk` → `disk`, `local_uptime` → `uptime`; `local_data` is gone (add the four checks). `ping` takes an IP address.
- **API changes:** `POST /api/silence` takes `check_id` instead of `resource_key`; silences have `scope_type` `check` and a `company_id`. `GET /api/probe/{id}/devices` answers `check_id`, `kind` (was `resource_type`) and `metrics` (was `metric_value`/`metric_unit`). `POST /api/probes/{id}` requires `interval_secs`.
- **Protocol v3 and new enrollment codes** (M2-03, D-48). Codes now carry the hub's signing key: create new codes after upgrading the hub, and use probes from this version to enroll. The hub accepts protocol versions 2 and 3; v1 reports (without `protocol_version`) are refused.
- **Probes join by enrollment** (M2-01, D-47). The hub's `[[probes]]` whitelist, `/api/handshake`, the probe's compiled-in `hub_cert.pem` (and `probe/build.rs`), and the probe settings `company_id`, `site`, `hub_url` and `hub_public_key` are gone. To upgrade: delete the hub database (migration `0006`), set the hub up again, add the companies and sites under **Companies**, then for each probe **Add probe** and run `probe enroll <code>` on it. `blentinel_probe.toml` keeps only `interval` and `[[resources]]`; the old keys are ignored, so delete them to avoid confusion.
- **The hub decides each probe's company and site;** the `company_id` and `site` in reports are ignored. Reports no longer create companies or sites (D-44's auto-registration): add them in the UI.
- **The web UI moves to HTTPS on port 3443** (M1-05, D-43). HTTPS is on by default, also for configs without `[server.tls]`: the hub creates a self-signed certificate, serves the UI and API on `https://<hub>:3443`, and keeps plain HTTP on `port` (3000) **only for the probe endpoints**, redirecting everything else to HTTPS. Existing probes keep working unchanged. Open port 3443 in the firewall; the browser warns about the self-signed certificate once (compare its fingerprint with the hub log). `https_port` now defaults to 3443, and "HTTPS only on `port`" is now written `https_port = <port>` (it used to be the absence of `https_port`). `enabled = false` still gives plain HTTP for everything.
- **API scripts must send `X-Blentinel: 1` on every POST** (CSRF protection); without it the hub answers `403 csrf`. Probes are unaffected.
- **User accounts replace the admin token** (M1-01, M1-02). `hub_auth.token`, the `auth_token_path` setting and `hub --reset-admin-token` are gone. The database upgrades itself (migration `0002`). After upgrading, start the hub, open the web UI, and create the first Owner with the **setup code** from the hub's log or `<state_dir>/setup.code`. Delete the old `hub_auth.token` file. An old config that still sets `auth_token_path` loads fine; the setting is ignored. Everyone who was logged in must log in again.
- **Delete the hub database before upgrading.** The schema is now managed by versioned migrations (`hub/migrations/sqlite/`). A database created by an earlier hub is refused at startup with instructions. Stop the hub, delete `blentinel.db` (and `-wal`/`-shm`), and start it again. Probe identities and the `[[probes]]` whitelist are unaffected.
- Upgrade the hub before the probes (protocol v2, see below).
- **New hub install layout** from `install_hub_service.sh`: program in `/opt/blentinel/hub`, config in `/etc/blentinel/blentinel_hub.toml`, data in `/var/lib/blentinel`. On Windows: a scheduled task, with the program in `C:\Program Files\Blentinel\Hub` and data in `C:\ProgramData\Blentinel\Hub`. Existing installs from the old scripts didn't work anyway (D-34); reinstall with the new script.

### Added (M2-15: probes update themselves from their hub, D-55)
- **Signed probe releases.** `blentinelmake release keygen` creates an Ed25519 release key on the build machine (the private key never goes to the hub); every hub and probe built afterwards trusts its public key. `blentinelmake release build --targets <triples>` builds the probes and signs them: `publish/probe-release-<version>/` (`manifest.json` with each binary's SHA-256, `manifest.json.sig`, the binaries) and a `.tar.gz`. `blentinelmake release sign <folder>` signs binaries built elsewhere.
- **The hub holds releases and offers one.** Add them on the admin page (**Probe updates > Add release**, the `.tar.gz`), with `hub release add <folder | file.tar.gz>`, or by installing a hub published with `--with-probes`; `hub release list` lists them. The hub refuses a release not signed by a key it trusts, with a binary that doesn't match the manifest, or for a protocol it doesn't speak. **No update is offered until an admin approves a release** (Probe updates > **Approve**; **Stop updates** withdraws it). The section lists the releases, how many probes run each, and every probe's version and last failed update. Add probe offers the approved release, else the newest.
- **Probes update themselves.** In its answer to a report, the hub offers the approved release to a probe running another version. The probe checks the release's signature against the keys compiled into it, refuses an older version (no downgrades), downloads its binary from the hub over its pinned connection while monitoring goes on, checks the SHA-256, and runs it with `--self-test` (it must start, read the probe's files and reach the hub). Only then does it switch: the old binary stays as `probe.prev`, and the service starts the new one (systemd: `Restart=always`; Windows: the service's recovery action; run by hand: the probe starts it).
- **Watchdog and rollback.** A new version must deliver a report within 15 minutes (or 3 intervals) and may start at most 3 times before it does; otherwise the probe puts `probe.prev` back and doesn't try that version again. The probe reports a refused or rolled-back update to the hub, which shows it on the admin page and the company's probe list and stops offering that version.
- **`probe updates off`** (the local emergency brake) refuses every update on that machine, and the hub shows why; `probe updates on` undoes it. `probe status` shows whether updates are possible, a version on trial and the last failure.
- API: `GET /api/admin/probe-updates`, `POST /api/admin/probe-updates/approve`, `POST /api/admin/probe-releases` (the `.tar.gz` as the body), `POST /api/admin/probe-releases/{version}/delete`, `GET /download/releases/{version}/{file}` (probes; no login, both ports). Probe records show `update_failed`; downloads show their `version`. Audit actions `release.add`, `release.approve`, `release.remove`. Migration `0008`.
- The Windows services (hub and probe) now also restart when they stop with an error code, not only when they crash.
- Tests: the manifest format, signatures (trusted, unknown key, tampered manifest or binary), version order; the probe's decisions (downgrade, brake, missing platform, giving up after 3 attempts), the watchdog (crash loop, report deadline), switching and rolling back the binary, a broken binary failing its self-test; the hub's release store (add, list, offer, archive with paths refused, a rebuilt version refused); the release API and its permissions, the offer in the answer to a report (approved, already installed, refused, v4 probe); storage conformance for the probe's version and the approval; a browser test of Probe updates. Verified end to end on a systemd host: a probe service updated itself from 0.1.0 to a signed 0.1.1, then was offered a 0.1.2 that crashes at start and went back to 0.1.1, keeping its identity.

### Changed (company page and enrollment codes, D-54)
- **Enrollment codes have no use limit by default**: one code sets up as many probes as needed until it expires (24 hours by default) or is revoked. **Limit** in Add probe (and `max_uses` in the API) still caps it; the API's default changes from 1 to no limit, and `max_uses` is `null` for codes without one. "Codes not used yet" is now **Codes still valid**, with the uses so far.
- **A company's details are shown as saved**, with **Edit** opening the form (Save or Cancel), instead of an always-open form.
- **Add probe is disabled until the company has a site**, with a hint to add one.

### Added (M2-09: one-line probe install, D-53)
- **Add probe gives a one-line command per platform**: it downloads the probe from the hub, checks its SHA-256 (shown on the hub's HTTPS page) and runs `probe install --code …`. Nothing runs if the download or the check fails. Linux uses `curl` and `sha256sum`; Windows, PowerShell as Administrator (5.1 or 7).
- **The hub serves probe binaries**: `GET /download/probe/{target}` (no login, on both ports) and `GET /api/probe-downloads` (the list with sizes and SHA-256). It offers what's in `<data folder>/probes/`; `hub install` copies the `probes/` folder beside it there, and `blentinelmake hub publish --with-probes <triples>` builds it. The answer to creating a code carries `hub_url`.
- **`probe install [--code <code>]`** (root/Administrator): copies itself into its folder (Linux `/var/lib/blentinel-probe`, Windows `C:\Program Files\Blentinel\Probe`), makes it private, enrolls with the code, registers and starts the service. Linux: a systemd unit registered over D-Bus (no `systemctl`), running as a throwaway user (`DynamicUser`) allowed to ping. Windows: the service `BlentinelProbe`, restarting on failure. Running it again upgrades in place, keeping the identity, enrollment, checks and buffer; with a new code it enrolls again.
- **`probe uninstall`** removes the service and the program and keeps the identity; `--delete-data` removes everything.
- **`probe status`**: service, folder, probe ID, hub, company and site, checks, reports waiting.
- **`probe doctor`**: checks the service, the folder (private), identity, enrollment, that the hub answers through its pinned certificate, the clock against the hub's, the checks and the buffer, with a fix for each problem; exits `1` on a failure.
- Probe builds for Linux are static (musl): one binary for any distribution.
- Tests: CLI parsing; the systemd unit; copying the probe into place; doctor's clock check against a test hub; downloads (listing, hashes, unknown targets, no login, the plain HTTP port); the one-line commands; a browser test of Add probe's command against the served file. Verified on a systemd host: the Linux line installs, enrolls and reports (ping as the throwaway user), upgrade, status, doctor, uninstall. The Windows line verified in PowerShell 5.1 and 7 (right hash runs; wrong hash or no hub runs nothing).

### Removed (M2-09)
- The probe's installer scripts (`install_probe_service.sh`, `.ps1`) and systemd unit template: `probe install` replaces them. A probe package is the binary and its checksum. Probes installed by the scripts (in `/opt/blentinel-probe` or by hand): stop and remove that service, then run `probe install --code <new code>`.

### Added (M2-08: the hub manages itself, D-52)
- **`hub install`** (root/Administrator): copies itself into place, creates the config on a first install (listening on all interfaces, ports 3000 and 3443; `--host`, `--port`, `--https-port`), makes the data folder private (Linux `0700` for the service user; **Windows: SYSTEM and Administrators only**), registers and starts the service, and prints the web address and setup code. Running it again upgrades: the program is replaced, the config and data kept. `--no-service` installs the files only.
- **The hub is a real Windows service** (`BlentinelHub`, automatic start, restarts on failure), logging to `hub.log` beside its config. It replaces the scheduled task.
- **`hub uninstall`** removes the service and the program; it **keeps the config and data** unless `--delete-data`.
- **`hub status`**: service, ports answering, web address, pending setup code, database size and schema, probes and users, certificate expiry.
- **`hub doctor`**: checks the config, service, data folder (exists, writable, private), identity key, database (opens, consistent, not from a newer hub), TLS certificate (expired, or within 30 days), ports and disk space, with a fix for each problem; exits `1` on a failure.
- **`hub backup [--out file]`**: a consistent copy of the database made while the hub runs, the identity key, TLS files and config in one `.tar.gz` (owner-only: it holds the private key). **`hub restore <file>`** (hub stopped) puts them where the current config says, moves the replaced files aside, keeps the backed-up config beside the current one, and refuses a backup from a newer hub.
- Without `--config`, these commands (and the hub) use `blentinel_hub.toml` in the working directory, else the installed hub's.
- Tests: CLI parsing; layout, systemd unit and install config; the Windows folder permissions applied and read back; doctor's checks (keys, certificates, ports, databases from a newer hub, disk space); backup → restore into another folder with users, probes and key; restore refusing a running hub, a newer backup or another file. Verified on a real systemd host: install, upgrade, status, doctor, backup, restore and uninstall.

### Removed (M2-08)
- The hub's installer scripts (`install_hub_service.sh`, `.ps1`), its systemd unit template and the sample config in the package: `hub install` replaces them. A hub package is the binary and its checksum.

### Changed (M2-07: a single-binary hub, D-51)
- **The hub is one file:** its web UI (WASM, JS, CSS, favicon) is embedded in the binary, so there's no `pkg/` folder to deploy and the hub runs from any folder. `blentinelmake hub publish` and the hub installers ship only the binary. Browsers revalidate the files by ETag (a `304` until the hub is upgraded).
- Building by hand takes two steps (front end, then server with `BLENTINEL_EMBED_SITE`); see CONTRIBUTING. `cargo leptos watch` still serves the files from disk, with live reload.
- Upgrading an installed hub: run the new installer; a `pkg/` folder left in the program folder by an earlier install is no longer used and can be deleted.
- Tests: the embedded files' lookup, types and caching; a build check that the installers copy no `pkg/`; the browser tests run the hub from a folder with nothing but its config.

### Added (M2-05: the probe page, D-50)
- **A page per probe** (`/probes/<id>`, from the probe's name on its company's pages): its checks with each one's latest result, **Add check** (a form showing the settings of the chosen kind, checked in the browser with the same code as the hub and the probe), **Edit**, **Disable/Enable**, **Delete**, the **report interval**, and a badge saying whether the probe runs the current checks (**In sync**, **Pending**, **Rejected** with the probe's reason). Refreshes every 15 s. Read-only for Viewers and Clients.
- **Copy checks from another probe**, for setting up a machine like one already monitored. API: `POST /api/probes/{id}/checks/copy`; audit action `check.copy`.
- Tests: the copy API and its scoping; the form's field mapping and validation; browser tests of the page (results, sync, add/edit/delete, interval refused with the reason, Viewer read-only).

### Added (M2-04: checks as hub records, D-49)
- **Checks are records on the hub,** each with a stable ID: renaming a check or changing its target keeps its history, alert state and silence (D-15). Kinds: `ping`, `http`, `tcp`, `cpu`, `memory`, `disk` (any mount point, or the system disk), `uptime`, with typed settings validated by one piece of code on the hub and the probe (`common::checks`).
- **The hub sends each probe its checks and report interval** in its signed answer to a report, whenever the probe runs another revision. The probe runs a new config at once, keeps it in `config.json`, and runs it after a restart or while the hub is down. A config it can't use is refused and reported; the probe keeps the previous one. No file is edited on the client's machine.
- **Several metrics per result:** `memory` reports usage %, used and total GB; `disk` also free GB; the first metric is the headline the UI shows. Thresholds apply to `usage`.
- **Each check is limited to 30 s,** so one slow check can't hold up a report.
- **A probe can only report on its own checks:** results for another probe's check (or a deleted one) are ignored.
- API: `GET|POST /api/probes/{id}/checks` (with the probe's sync state: `in_sync`, `pending`, `rejected` and why), `POST /api/checks/{id}`, `POST /api/checks/{id}/delete`, and `interval_secs` on `POST /api/probes/{id}` (5–3600 s). Audit actions `check.create`, `check.update`, `check.delete`; a "Checks" filter on the Audit page. Reference: [docs/reference/checks.md](docs/reference/checks.md).
- Tests: `CheckSpec` validation and config rules (`common`); config revision, sending and refusal rules (hub service); the check API, config push, sync state, refusal and cross-probe results end to end (`hub/tests/checks.rs`); storage contracts for checks, metrics and config state; the probe's config cache and checks; a browser test with the real probe running the checks the hub gave it.

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
