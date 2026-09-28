# Blentinel — Decision Log & Challenged Assumptions

This file records design decisions and **challenges to earlier specs, docs, and assumptions**. Each entry has a status:

- **Proposed:** a recommendation that needs the project owner's approval.
- **Accepted:** agreed; the roadmap reflects it.
- **Rejected:** considered and declined, with the reason recorded.

Newest entries at the bottom. Never delete an entry. Supersede it with a new one.

---

## D-01 · Hub TLS certificate embedded in the probe at compile time → pin at enrollment
**Status:** Proposed · **Affects:** README "HTTPS Configuration", `probe/build.rs`, HTTPS_SETUP_GUIDE.md · **Roadmap:** M2

**Today:** `probe/hub_cert.pem` is compiled into the probe binary. Every hub needs its own probe build, and a cert change means rebuilding and redeploying every probe.

**Challenge:** this contradicts PP-1 (trivial install) and makes renewing the certificate painful. It doesn't add security over pinning at runtime, because both approaches pin a single fingerprint.

**Decision:** the probe pins the **SHA-256 fingerprint of the hub's TLS public key**, which it receives in the enrollment code (proposal 0002). Everyone uses the same official probe binary. Certificate rotation: the hub announces the *next* fingerprint in report responses before switching, so probes accept both during an overlap window. FP-3 is preserved.

## D-02 · Unauthenticated `/api/handshake` key fetch → hub key comes from the enrollment code
**Status:** Proposed · **Affects:** `probe/src/transport.rs::fetch_hub_pk`, `agent.hub_public_key` · **Roadmap:** M2

**Today:** if `hub_public_key` is not set, the probe fetches the hub's X25519 key over `/api/handshake` and trusts whatever it gets back (trust on first use, with no verification). An attacker on the network path at first contact could impersonate the hub.

**Decision:** the enrollment code carries the hub key fingerprint. The probe verifies the handshake result against it and refuses to proceed on mismatch.

## D-03 · HTTP as the default transport → HTTPS by default for the web UI
**Status:** Proposed · **Roadmap:** M1

**Today:** HTTP is the default. Probe payloads are still protected because they are encrypted at the application layer, but the **web UI is not**.

**Challenge:** once usernames and passwords exist (proposal 0001), plain HTTP exposes credentials and session cookies.

**Decision:** the hub serves **HTTPS by default** with an auto-generated self-signed cert, optional ACME/Let's Encrypt, or documented reverse-proxy mode. Plain HTTP stays available only as an explicit opt-in (e.g. behind a TLS-terminating proxy). Session cookies are marked `Secure`.

## D-04 · Probe whitelist and alert settings in `blentinel_hub.toml` → database + UI
**Status:** Proposed · **Affects:** `[[probes]]`, `[alerts]`, hub hot-reload · **Roadmap:** M1/M2

**Challenge:** editing TOML on the server to add a probe or technician violates PP-2.

**Decision:** probes, companies, sites, technicians and users, alert channels, and thresholds move to the database and are managed in the UI. The TOML keeps **bootstrap only**: bind address and ports, data directory, TLS file paths. On first start after the upgrade, existing `[[probes]]` and `[alerts]` entries are **imported once**. After that the sections are ignored with a warning. Hub config hot-reload shrinks accordingly (possibly to nothing).

## D-05 · Probe-local `[[resources]]` as the source of truth → hub-managed configuration
**Status:** Proposed · **Affects:** `blentinel_probe.toml`, probe hot-reload · **Roadmap:** M2

**Challenge:** changing a check today means remote access to the client's machine, which violates PP-2.

**Decision:** checks are defined in the hub and delivered to the probe **in the response to its report** (FP-1 compliant; proposal 0003). The probe caches the last good config on disk (FP-4). The local TOML shrinks to bootstrap: hub URL(s) and data directory. **No local check overrides**: one source of truth. A `--standalone <file>` developer mode may remain for testing.

## D-06 · Shared admin token → per-user accounts
**Status:** Accepted (requested by owner) · **Roadmap:** M1 · proposal 0001

The token becomes a **break-glass CLI path only**: `hub user reset-password <name>`, run on the server.

## D-07 · `Cargo.lock` is git-ignored → commit it
**Status:** Accepted (2026-09-27). The owner commits `Cargo.lock` and adjusts `.gitignore`. · **Roadmap:** M0

**Today:** `.gitignore` excludes `Cargo.lock`, so every build resolves "latest compatible" versions. Builds aren't reproducible, a release can silently pick up a broken or compromised dependency, and CI can't bisect.

**Decision:** commit `Cargo.lock`. This is Cargo's official guidance for applications.

## D-08 · Docs referenced by the README are git-ignored
**Status:** **Done** (M0-12, 2026-09-27): the content lives in `docs/`, `PROGRESS.md` and `tree.txt` are retired, and nothing links to the old files. Any copies left in a working folder are untracked and git-ignored, and can be deleted. · **Roadmap:** M0

**Today:** `.gitignore` excludes `HTTPS_SETUP_GUIDE.md`, `HTTPS_IMPLEMENTATION.md`, `HOT_RELOAD_TESTING.md`, `PROGRESS.md`, and `tree.txt`. The README links to the first three, so those links are **broken for anyone who clones the repo**.

**Decision:** move the still-valid content into `docs/` (`install/hub.md`, `security.md`, `architecture.md`) and commit it. **Retire `PROGRESS.md`.** It is badly outdated: it says the whitelist doesn't exist, tables aren't created, the web UI is a demo counter, and the hub key regenerates on restart. None of that is true any more. Its still-open items are folded into `ROADMAP.md`. Retire `tree.txt` (the README already has a tree, which will also be removed as noise).

## D-09 · LaTeX manual duplicates the README → single Markdown source
**Status:** **Implemented** (2026-09-27, M0-12/M0-15) · **Affects:** `manual/manual.tex` (1,938 lines, retired) · **Roadmap:** M0-15

**As built:** pandoc's standard LaTeX template plus `docs/manual/header.tex` (the old preamble's page header and colours) instead of a fully custom template, which is more robust across pandoc versions. Chapter order: `docs/manual/chapters.txt`. Title page: `docs/manual/manual.yaml` (version taken from the workspace). Command: `blentinelmake docs`. The manual's content was reviewed section by section and moved into `docs/`, correcting what had become wrong: the "perfect forward secrecy" claim, silence scopes, buffer behaviour, CLI flags, status codes, the hot-reload claims, and the cross-compile claim.

**Challenge:** keeping two hand-maintained sources (README and manual.tex) in sync violates DRY and PP-4, so they will drift.

**Decision:** `docs/` Markdown is the **single source of truth**. The LaTeX manual is **generated** from it, not written by hand:

- `pandoc` converts the Markdown to LaTeX using a **custom template** built from the current `manual.tex` preamble (report class, `fancyhdr` header "Blentinel Technical Manual", `listings` code style, `booktabs` tables, title page, TOC). The PDF keeps its current look.
- Chapter order and title metadata live in one small file (`docs/manual/manual.yaml`). The command is roughly: `pandoc --template docs/manual/template.tex --listings --top-level-division=chapter --toc <ordered docs> -o manual.tex` → `xelatex`.
- Content that exists only in today's `manual.tex` is migrated into `docs/` **once**. `pandoc manual.tex -o …md` gives a first draft to split into the right pages.
- The generated `.tex` and `.pdf` are build outputs: not committed, or attached to releases. A `blentinelmake docs` target runs the build locally. CI integration is left to the owner.

## D-10 · Suggested move to TimescaleDB/InfluxDB → rejected; SQLite + rollups
**Status:** Superseded by D-22 (the rejection of a *required* time-series DB stands)

**Context:** `PROGRESS.md` recommends a dedicated time-series database.

**Decision:** **rejected.** It would add an external service and break PP-1 and PP-3. Growth is handled with raw-data retention plus hourly and daily **rollups** in SQLite (proposal 0007), which keeps the database small and long-range charts fast. The repository layer is kept behind traits so that an optional Postgres backend *could* be added later without affecting anything else.

## D-11 · "The hub cannot be cross-compiled because of Leptos" → incorrect; single-binary hub
**Status:** Accepted. README corrected (M0-12, 2026-09-27); embedding the web assets in the hub binary is M2-07 · **Affects:** README "Cross-Compilation" · **Roadmap:** M2

**Challenge:** the WASM front end is architecture-independent. Only the server binary is platform-specific, and it can be cross-compiled (`cargo-zigbuild`/`cross`) or built natively on CI runners per OS. The real deployment pain is that the hub needs its `site/` directory shipped beside the binary.

**Decision:** embed the web assets in the hub binary (e.g. `rust-embed`) → **one file to deploy**. CI builds per-OS natively. Fix the README statement.

## D-12 · "Smooth UI polling" spec (2026-03-18) is superseded → push updates via SSE
**Status:** Proposed · **Affects:** `docs/superpowers/specs/2026-03-18-smooth-ui-polling-design.md` · **Roadmap:** M6

**Context:** that spec prescribed `<Suspense>` → `<Transition>`. The code has since moved to `RwSignal` + `Memo` + `spawn_local`, and commit history shows the flashing was "fixed" repeatedly (`a67ed46 fixing flashing... again, and again`).

**Challenge:** polling every 15 seconds and diffing on the client treats the symptom.

**Decision:** mark the old spec **Superseded**. The hub pushes changes to the browser over **Server-Sent Events** (`/api/events`), and the UI applies deltas. This is compatible with FP-1, which only restricts hub → probe connections. Polling remains as a fallback.

## D-13 · Linux probe built for `*-linux-gnu` → `*-linux-musl` static
**Status:** Accepted. Linux probes are built for musl (CONTRIBUTING, `blentinelmake probe publish --target …-musl`) and were verified `static-pie linked` (D-34 addendum, 2026-09-27); the automatic check in CI comes with M0-13 · **Roadmap:** M2

**Checked:** the probe already uses rustls (no OpenSSL) and bundled SQLite. That's good. But the `gnu` target still links dynamically against glibc, so a binary built on a new distro fails on older ones. This is a real FP-2 gap.

**Decision:** release builds for Linux use the musl targets (`x86_64`, `aarch64`, `armv7`). CI verifies with `ldd` or `file` that the binaries are static.

## D-14 · Mobile app spec relies on the web session cookie → per-user API tokens
**Status:** Proposed · **Affects:** `docs/proposals/0010-mobile-app.md` · **Roadmap:** separate project (see D-23)

The mobile app is a **separate, pending project** (D-23). When resumed, it authenticates with per-user API tokens or its own sessions (proposal 0001), not the shared admin cookie.

## D-15 · String resource key `company:probe:name:target` → stable `CheckId`
**Status:** Proposed · **Roadmap:** M2 (with 0003)

**Challenge:** identity built from mutable text means renaming a check or changing its target loses its history, silences, and alert state, and a `:` inside a name breaks parsing.

**Decision:** checks become hub records with a UUID `CheckId` (proposal 0003). Results, silences, incidents, and alert state reference `CheckId`.

## D-16 · Binary Up/Down status → Up / Warning / Down / Unknown
**Status:** Proposed · **Roadmap:** M3 · proposal 0005

**Today:** threshold breaches (disk ≥ 90%, etc.) only trigger an e-mail. The UI still shows the resource as **Up**, and a probe's stale devices are also shown as their last state.

**Decision:** four states, evaluated **once, on the hub** (a single source of truth). *Warning* covers threshold breaches. *Unknown* covers stale data or an unreachable parent dependency.

## D-17 · Probe-reported `company_id` is trusted → hub assigns company and site
**Status:** Proposed · **Roadmap:** M2 · proposal 0002

**Today:** any whitelisted probe can report under **any** `company_id` string, and a typo creates a new "company".

**Decision:** company and site come from the hub's probe record (FP-3). The field in reports is deprecated and then ignored.

## D-18 · Protocol has no version field (fixed: protocol v2, M0-06)
**Status:** **Done** (M0-06, 2026-09-27): protocol v2, signed `protocol_version` in reports, `X-Blentinel-Protocol` header · **Roadmap:** M0

`common/src/models.rs` warns "do not reorder fields without bumping protocol version", but no version field exists. Add `protocol_version` to the report and its response (ENGINEERING §2.7) **before** the M2 protocol changes.

## D-19 · Woodpecker CI → GitHub Actions
**Status:** Accepted in principle (from TODO.md). **On hold: CI changes are owner-controlled (D-24).** · **Roadmap:** M0-03

The repo has moved to GitHub, but `.woodpecker.yml` only builds the hub. Replace it with the GitHub Actions pipeline in ENGINEERING §5, then delete `.woodpecker.yml`. The README clone URL still points to Codeberg; update it.

## D-20 · Archive viewer → keep until rollups ship, then reassess
**Status:** Superseded by D-39 (archiving stays) · **Roadmap:** M6

Archiving exists because every raw check result is kept forever. Once retention and rollups ship (0007), long-range history is available without archives. At that point, decide whether archive/export stays as a **compliance export** feature or is removed.

## D-21 · Build tool vs. CI duplication
**Status:** Accepted: it is the rule in ENGINEERING §2.3 ("Build steps: `blentinelmake`; CI invokes it"), applied once CI is enabled (D-24) · **Roadmap:** M0

`blentinelmake` replaced the PowerShell scripts (good). CI must **call `blentinelmake`** rather than repeat its steps, so there is one build definition (DRY).

## D-22 · Storage: embedded by default, pluggable external backends
**Status:** Accepted in principle (owner, 2026-09-27) · **Supersedes:** D-10 · **Roadmap:** M0-14, M7 · proposal [0011](proposals/0011-storage-backends.md)

**Owner input:** "I really don't want to have to install a separate database … ideally the DB is configurable (SQLite or other, PostgreSQL, Oracle, SQL Server …), modular, with an embedded DB by default."

**Decision:**

- **SQLite stays the embedded default.** Other embedded engines were evaluated (libSQL, DuckDB, redb/fjall/sled, SurrealDB, Limbo; see 0011 §2). None beats SQLite for this workload, which has ~100× headroom at the target scale. libSQL is on the watch list.
- The hub's storage becomes **modular**: domain-level repository traits (`storage::Storage`). Each backend is a separate crate behind a feature flag and must pass a **shared conformance test suite**.
- **Optional backends are future specs:** PostgreSQL (P2), then MySQL/MariaDB and SQL Server (P3). Oracle (P3, on request) needs Oracle's native client libraries, so it ships only as a **separate build flavour** and never in the default single binary (PP-1).
- A dedicated time-series DB is still **not required**. Retention + rollups (0007) are implemented against the traits, so they work on every backend.
- PP-3 is reworded to "embedded by default, external optional, never required".

## D-23 · Product website and mobile app are separate projects
**Status:** Accepted (owner, 2026-09-27) · **Roadmap:** "Separate projects" section

The **product/marketing page** (the old TODO "Create web page") and the **mobile app** (proposal 0010) are not part of the probe–hub core. They are tracked as separate modules/projects with status *pending*. They consume the hub's public API and the docs, and are not scheduled in the core milestones.

## D-24 · CI is owner-controlled for now
**Status:** Accepted (owner, 2026-09-27) · **Roadmap:** M0-03 on hold

Do not create or modify CI configuration (`.woodpecker.yml`, `.github/workflows/*`) until the owner decides. Proposals may describe the target pipeline (ENGINEERING §5), but local commands (`cargo fmt/clippy/test`, `blentinelmake`) are the enforcement mechanism until then.

## D-25 · Unsigned and replayed reports are accepted (security)
**Status:** **Fixed.** Part 1 in M0-16 (signature required), part 2 in M0-17 (replay protection, protocol v2), both 2026-09-27. · **Found by:** M0-04 characterisation tests · **Roadmap:** M0-16, M0-17

**Found:** `ingest::handle_probe_report` only verifies the signature *if one is present*. A report with `signature: None` from any whitelisted probe ID is stored. A probe's ID is its **public** key, and the hub's X25519 key is served by `/api/handshake`, so anyone who can reach the hub can forge reports for any registered probe. There is also **no replay protection**: a captured sealed report can be re-sent and is stored again.

**Evidence:** `hub/tests/ingest.rs` → `known_issue_unsigned_report_is_forbidden` (gets 200, expected 403) and `known_issue_replayed_report_is_rejected` (gets 200 twice).

**Decision (proposed):**

1. **Require a signature** (M0-16, P0, S). The real probe always signs (`probe/src/main.rs`), so this breaks nothing.
2. **Reject replays** (M0-17, P1, M): reject reports whose `timestamp` is outside a window around the hub clock (e.g. ±10 min, allowing for buffered reports being flushed in order), plus a per-probe "last accepted timestamp" high-water mark. Buffered offline reports (FP-4) arrive in order, so they still pass the high-water mark. Final design lands with the M2 protocol envelope (`protocol_version`, D-18).

## D-26 · Archive viewer shows (almost) nothing
**Status:** Decided by D-39: fix it (M0-19) · **Found by:** M0-04 characterisation tests · **Roadmap:** M0-19 (or fold into D-20)

**Found:**

- Archiving prunes `probe_heartbeats` rows whose `last_seen_at` is after the cutoff. The archive dashboard groups by heartbeat, so any probe **still reporting today disappears from the archive**, even though its old reports are in it.
- Archive uptime asks for "the last 24 h relative to now" (and ignores the `range` parameter), but an archive only holds data older than its cutoff, so it is **always empty**.

**Evidence:** `hub/tests/admin.rs` → `known_issue_archive_shows_companies_whose_probes_are_still_reporting`, `known_issue_archive_uptime_contains_archived_data`.

**Decision (proposed):** because D-20 may retire the archive viewer once rollups exist, fix cheaply now only if archives are used in practice (P2): keep heartbeats in the archive and compute uptime relative to the archive's cutoff date. Otherwise record it as a known limitation until D-20 is decided.

## D-27 · A probe that goes offline twice only alerts once
**Status:** **Fixed** (M0-18, 2026-09-27): each report re-arms the probe's expiry alert (`alerts::state::rearm_probe_expiry_alert`). · **Found by:** M0-04 characterisation tests · **Roadmap:** M0-18

**Found:** when the "probe offline" e-mail is sent, alert state `probe_expiry:<company>:<probe>` becomes `expired`. Nothing resets it when the probe reports again, so the next outage is suppressed (`should_send_alert` sees no change). Silent failure of the main thing monitoring is for.

**Evidence:** `hub/tests/expiry.rs` → `known_issue_probe_that_expires_twice_alerts_twice`.

**Decision (proposed):** reset the expiry state (to `active`) when a report arrives from that probe (M0-18, P1, S). Superseded in M3 by incidents (proposal 0005), but it's too serious to wait for that.

## D-28 · Protocol v2: versioning and replay protection (implements D-18, D-25 part 2)
**Status:** Accepted (implemented 2026-09-27, M0-06 + M0-17)

**Versioning:** reports carry a **signed** `protocol_version` (currently 2). It is *omitted* for v1, so a legacy report serialises to exactly the bytes old probes signed and their signatures still verify. The hub accepts versions 1..=2 and answers anything else with 400. Every probe-facing response carries `X-Blentinel-Protocol`. The constants live in `common::protocol`, the single source for both sides.

**Replay protection (hub):**

1. The timestamp must be no more than 5 min in the future and no older than 7 days + 5 min (`MAX_BUFFERED_REPORT_AGE_SECS` + `MAX_CLOCK_SKEW_SECS`), otherwise **422**.
2. It must be **strictly newer** than the probe's latest stored report, otherwise **409**. The check and the insert are one SQL statement (`INSERT … SELECT … WHERE NOT EXISTS`), so concurrent duplicates can't both be stored. New index `reports(probe_id, timestamp)`.
3. Both checks run **after** signature verification, so forged traffic learns nothing about the window.

**Known limitations (accepted for now):**

- A probe clock more than 5 min fast gets every report rejected (422, logged on both sides). Hosts should run NTP. `hub_time` in the future `ReportAck` (proposal 0003) will let probes detect and report skew.
- "Delete probe data" in the admin UI also removes the replay high-water mark, so reports captured in the last 7 days could be replayed once after such a deletion. It's rare and admin-triggered. It goes away when M2 moves probes to records with their own `last_report_at`.

## D-29 · Probe offline buffer: store only undelivered reports, bounded, in order
**Status:** Accepted (owner requirement, implemented 2026-09-27) · **Code:** `probe/src/outbox.rs`

**Owner requirement:** "the client should not store data unless it has not been sent; once sent to the hub it should be deleted".

**Found (before the fix):**

- **Any** failed send was buffered, including permanent rejections (e.g. 403 before the probe is registered). The flush stopped at the first failure, so one rejected report blocked the queue forever and it **grew without limit**.
- There was no age or size limit.
- The current report was sent *before* the backlog (out of order), which is incompatible with replay protection.

**Decision (implemented):**

- The hub has it (200, or 409 "already have it") → deleted. A report is written to disk **only** when delivery fails transiently.
- Permanent rejections (other 4xx) → dropped and logged as errors, never buffered.
- Transient (network, timeout, 5xx, 408, 429) → kept.
- **In-order delivery:** while a backlog exists, new reports queue behind it. Up to 500 are flushed per cycle, oldest first.
- **Bounds:** reports older than `MAX_BUFFERED_REPORT_AGE_SECS` (7 days, the same constant the hub enforces) are pruned, and at most 20,000 are kept (oldest dropped first).

Verified by 11 unit tests plus an end-to-end run with the real binaries (hub down → buffered, hub back → flushed in order, buffer empty, zero hub rejections).

## D-30 · Pre-alpha: no backward compatibility or data migration
**Status:** Accepted (owner, 2026-09-27) · **Revisit:** before anyone other than the owner runs Blentinel

**Owner input:** "backwards compatibility, as of now, is not required … we can scrap production data if needed, don't over-complicate things trying to migrate DBs or keep stale data … we are still in pre-alpha."

**Decision:** until the first external user, breaking changes to the database, config files, and probe↔hub protocol are allowed. Upgrades may require **deleting the hub database and probe state and re-registering probes**. The CHANGELOG says so under a "Breaking" heading.

**Planned work this removes or simplifies:**

- M0-05: migrations start from a fresh baseline. No adoption of existing databases.
- M2-02 and proposal 0002 §4.6: no import of the `[[probes]]` TOML or existing companies. Re-enroll probes.
- Proposal 0003 §4.4: no `[[resources]]` import or history mapping. Recreate checks in the UI.
- D-04: no one-time import of `[alerts]` or `[[probes]]` into the DB.
- Proposal 0001 §4.4: no migration from the admin token beyond the first-run wizard.
- Protocol: the v1 compatibility already built (D-28) stays because it costs nothing, but future protocol changes don't need to keep old probes working.

**Still required:** versioned migrations themselves (M0-05), so that schema changes are repeatable and tested. This decision is about not preserving *old* data, not about skipping migrations.

## D-31 · One severity rule, computed by the server
**Status:** Accepted (implemented 2026-09-27, M0-08) · **Code:** `hub/src/model.rs` (`Severity`)

**Found:** severity was written three times in UI code with two contradicting rules. The dashboard **sorted** a company as CRITICAL when any device was down, even stale ones from an expired probe. The company **card** coloured it amber whenever any probe had expired, which **hid a live outage** on another probe.

**Decision:** a company is as bad as its **worst probe**:

- **Probe:** expired → `degraded` (its device counts are stale). Active with devices down → `critical`. Otherwise `healthy`.
- **Company:** any device down on an *active* probe → `critical`. Otherwise any expired probe → `degraded`. Otherwise `healthy`.

The server computes it (only the server knows which down devices belong to active probes) and returns it as `severity` in the company and probe JSON. The UI only maps it to labels and colours, and API clients (mobile app) get the same answer. `Severity`'s `Ord` sorts worst first. Superseded in M3 by the 4-state model (D-16), which extends this enum rather than replacing the approach.

## D-32 · Storage backends are modules until there is a second backend
**Status:** Accepted (2026-09-27, M0-14) · **Refines:** proposal 0011 §3

Proposal 0011 sketched one crate per backend (`storage/`, `storage-sqlite/`, …). With a single backend, separate crates only add plumbing (the hub's `ssr`/`hydrate` feature split, workspace wiring) and no benefit. So:

- The traits live in `hub/src/storage/mod.rs` and SQLite in `hub/src/storage/sqlite/`. SQL is private to that module (`queries.rs`), and nothing else in the hub touches `sqlx` (verified by grep).
- `AppState` holds `Arc<dyn Storage>`. Business code needing one area depends on the narrow trait (e.g. `alerts` takes `&dyn AlertStore`).
- The conformance suite (`hub/tests/storage_conformance.rs`) is a macro, so each new backend adds one line.
- **Split into crates when the second backend (PostgreSQL, M7) starts**, so optional backends can be feature-gated out of the default binary.
- Archiving is part of the storage contract (`ArchiveStore`) because it is inherently backend-specific (SQLite copies its file). D-20 may still retire it.

## D-33 · Metric thresholds never alert; silences survive recovery
**Status:** **Fixed** (M0-21, 2026-09-27) · **Found by:** M0-09 step 3 (first alert tests, made possible by the `Notifier` trait)

**Found (both pinned by ignored `known_issue_*` tests in `hub/tests/alerts.rs`):**

1. **Disk/CPU/memory threshold alerts effectively never fire.** In `service/alerts/engine.rs`, the threshold check runs *after* an early return taken whenever the resource's Up/Down status is unchanged. Local metrics are always reported as Up, so a disk steady at 95 % never reaches the check. It could only fire on a Down→Up recovery. The threshold state key (`<key>-threshold`) is also never reset, so even that would alert once, ever (the same bug class as D-27).
2. **"Clear the silence when the resource recovers" is unreachable.** Silenced resources skip evaluation entirely, so a silence outlives the outage it was created for (until it expires, or forever).

**Decision (proposed):** evaluate thresholds independently of Up/Down transitions, as their own state (`ok` ↔ `exceeded`) that alerts on entry and resets on exit. On recovery, clear the silence *before* the silenced-skip. Both are small fixes. They could also be folded into M3's alert state machine (proposal 0005), but a disk-full alert that never fires is too important to wait.

**Fix (M0-21):**

- **Thresholds** are a separate state per resource (`<key>-threshold`: `threshold_exceeded` ↔ `threshold_ok`), evaluated on every report regardless of Up/Down. They alert when the metric crosses the limit, stay quiet while it stays over, and re-arm when it drops below. The state is written only when the metric crosses the limit or drops back, never for metrics that stay normal. A failed send is retried on the next report.
- **Silences** end when the resource recovers, even while silenced. No recovery alert is sent for an outage nobody was alerted about. A silence on a resource that stays healthy is kept. (As designed originally, *every* silence ends on recovery. Proposal 0005 adds explicit "until recovery" vs. timed silences.)

**Addendum (2026-09-27): a race left D-27 open.** The expiry alert recorded "expired" *after* sending. A probe that reported again while the e-mail was being sent was re-armed, then overwritten back to "expired", so its next outage never alerted. It was found through a flaky test and reproduced deterministically with a notifier that blocks until released (`hub/tests/alerts.rs`). Fix: record the alert before sending. Nothing is lost, because an expired probe isn't re-reported by later sweeps, so a failed send was never retried anyway.

## D-34 · Shipped service installers are broken
**Status:** **Fixed** (M0-22, 2026-09-27), except item 4 (tracked as M0-23) · **Found by:** M0-12 (checking the install docs against the real packages)

**Found** (generated by `blentinelmake`, `generate_*_service_files`):

1. **Hub, Linux:** `blentinel-hub.service` runs as `User=blentinel`, but `install_hub_service.sh` never creates the user or gives it the files, so the service fails to start. The script also does `cp *` from the package folder, so a package that has been run once ships its `hub_auth.token`, `hub_identity.key` and database to the server.
2. **Hub, Windows:** `install_hub_service.ps1` registers `hub.exe` as a Windows service, but the hub has no service mode (only the probe does), so it fails to start (error 1053). It also doesn't copy the `pkg/` web assets.
3. **Probe, Linux:** `User=blentinel # change as needed`. systemd doesn't allow comments after a value, so the user name is invalid. The user isn't created and `cap_net_raw` isn't set (ping fails without root). Worst, the script's `rsync --delete ./ /opt/blentinel-probe/` **deletes the probe's `identity.key` on every upgrade**, giving it a new ID that must be re-registered.
4. **Probe hot reload** (docs only): the docs claimed every probe setting reloads, but `hub_url` and `hub_public_key` are read only at startup.

**Workarounds** are documented in `docs/install/hub.md` and `docs/install/probe.md`. **Fix (M0-22, proposed):** create the user and set ownership; set `cap_net_raw`; never delete files the package doesn't contain; refuse to publish a package folder that contains runtime files; give the hub a Windows service mode (as the probe has), or ship a scheduled-task installer. The proper answer is `hub install` / `probe install` (proposal 0004, M2-08/M2-09), which should replace these scripts.

**Fix (M0-22):**

- Installers are real script files in `blentinelmake/templates/`, embedded with `include_str!`, written with LF endings, and covered by regression tests (no inline comments in units, a `--config` the parser accepts, service user created, no whole-folder copies, the probe's identity and config never touched).
- **Hub, Linux:** program `/opt/blentinel/hub`, config `/etc/blentinel/blentinel_hub.toml` (first install only), state `/var/lib/blentinel` (never touched; also the working directory, so archives and TLS files land there). The hub now finds its web assets next to its executable, which makes that possible.
- **Hub, Windows:** a scheduled task (at boot, SYSTEM, restart on failure) instead of a broken service. Program in `C:\Program Files\Blentinel\Hub`, config and data in `C:\ProgramData\Blentinel\Hub`. A real Windows-service mode is still desirable (M2-08).
- **Probe, Linux:** upgrades replace only the binary; the config is written on first install only; ping works through `AmbientCapabilities=CAP_NET_RAW`, which, unlike `setcap`, survives binary replacement.
- Verified: unit tests, `bash -n`, the PowerShell parser.

**Addendum (2026-09-27): verified on a real systemd host.** An Ubuntu 24.04 container with systemd as PID 1 ran both installers with Linux builds (the probe for `x86_64-unknown-linux-musl`: `static-pie linked`, so FP-2 holds). All checks passed (34). **Hub:** service user created; service active; `/var/lib/blentinel` is `0700 blentinel` and holds the key, token and database; config points at it. An upgrade with a new binary and stray `hub_auth.token`/`hub_identity.key` files in the package folder replaced the binary, kept the key, token and edited config, and copied neither stray file. **Probe:** enabled but not started on first install; `sudo -u blentinel …/probe` printed its ID; after registration the service ran as `blentinel` with **only** `cap_net_raw` (ambient; no file capabilities), and a ping check reported *Up*, so raw sockets work without root. An upgrade kept `identity.key` (same hash) and the edited config, and reports continued under the same probe ID.

**Found while verifying:** every secret file (`identity.key`, `hub_identity.key`, `hub_auth.token`, the TLS key) was written with the default umask: `0644`, or `0664` when created from a shell. The installers' `0700`/`0750` folders kept other users out, but a hub or probe started by hand left private keys world-readable. **Fix:** `common::fs::write_secret_file` creates them as `0600` (and tightens an existing file it overwrites). Windows inherits folder ACLs, which let local users read by default. That is documented in `security.md`, and fixing it is part of M2-08.

## D-35 · One error envelope for the web API and the probe protocol
**Status:** Accepted (2026-09-27, M0-11) · **Implements:** ENGINEERING §2.5

Every hub error response is `{"error": {"code", "message"}}`: the web API, the archive viewer, and `POST /api/report`.

- **One type, in `common`:** `common::protocol::ErrorBody`. The hub serialises it (`hub/src/error.rs`, `ApiError: IntoResponse`), the web UI shows its `message`, and the probe logs it. It sits in `common` without the `crypto` feature, so the WASM UI and the probe can use it too.
- **The probe still decides by status code alone** (200/409 delivered, other 4xx dropped, 5xx retried; D-29). The body only explains the decision, so a proxy's HTML error page changes nothing. Adding the body didn't change the protocol version.
- **Codes are stable, messages are not.** Scripts match on `code`. The codes are listed in `reference/api.md` and `reference/protocol.md`, and pinned by `hub/tests/errors.rs`.
- **Internal errors never carry details** (`internal`, "see the hub log"). The cause is logged where it is converted (`ApiError::internal`), so paths and SQL stay on the server.
- **Probe rejections are specific** (`unknown_probe`, `bad_signature`, …). They only tell the sender about its own report. Knowing whether a public key is registered doesn't help anyone who can't sign for it.
- **Logging:** the hub uses `tracing`. The default level is `info`; `--verbose` means `debug` and `--debug` means `trace` for the hub's own crates, other crates log at `warn`, and `RUST_LOG` overrides everything. The one-shot CLI commands (`--init`, `--reset-admin-token`, `--print-public-key`, the first-run banners) still print to the terminal, because that output is the command's result.

## D-36 · The hub must run on Linux; Windows is best effort
**Status:** Accepted (2026-09-27, owner)

The hub's supported platform is **Linux** (x86_64 and ARM64): releases, installers and tests must always work there. Windows is **best effort**: keep it working when it's cheap (it builds and passes the tests today, and there is a scheduled-task installer), but a Windows-only problem never blocks a release, and features may land on Linux first. The probe is unaffected: it stays first-class on Linux and Windows (FP-2).

Background: the hub was remembered as documented "Linux only". A likely reason is that building its web UI on Windows is awkward: `cargo install cargo-leptos` compiles OpenSSL, which needs Strawberry Perl (the `perl` bundled with Git for Windows lacks modules). CONTRIBUTING documents this; building in Docker or WSL also works.

## D-37 · What `common` shares between the probe and the hub (M0-07)
**Status:** Accepted (2026-09-27, M0-07) · **Refines:** ENGINEERING §2.3

ENGINEERING §2.3 listed "crypto, identity, TLS pinning, hot reload, arg parsing" as copies to move into `common`. Comparing the code showed which ones really were copies:

- **Moved:** crypto (`common::crypto`, M0-04); loading or creating 32-byte key files and writing secrets owner-only (`common::fs`: the probe's Ed25519 identity and the hub's X25519 key had the same load, length-check and save code); the config-file watcher (`common::watch`, about 45 identical lines); the flags both CLIs accept (`common::cli`: `--help`, `--version`, `--init`, `-v`, `-d`, and the unknown-option error). Each binary keeps what is its own: key types, what a reload does, its extra CLI options.
- **Not moved: TLS.** The probe *pins* a certificate embedded at build time (a `reqwest` client). The hub *generates* a self-signed certificate and serves it (`rcgen`, `axum-server`). They share no code, and a common module would only couple two unrelated dependency sets. TLS pinning changes with enrollment anyway (D-01).
- **CLI parsing stays hand-written and dependency-free** (FP-2: the probe binary stays small). It is pure and tested; only `main` prints and exits.
- **`common` stays light for the WASM UI:** crypto and the watcher are behind features (`crypto`, `watch`); `cli` and `fs` use only `std`.

## D-38 · Every config setting reloads without a restart (M0-23)
**Status:** Accepted (2026-09-27, owner request) · **Supersedes:** the "needs a restart" list in `reference/configuration.md`

Editing `blentinel_hub.toml` or `blentinel_probe.toml` takes effect without a restart. How:

- **A reload is all-or-nothing.** The new file is validated, then everything it needs is prepared *before* anything is swapped: open the database at the new path, load or create the key and token at new paths, check that a new address can be bound. If any step fails, the error is logged and the hub keeps running with the old file's settings **and** resources. Only then are the config, the database, the hub key and the listeners swapped.
- **Database (`state_dir`, `db_path`):** the hub switches to the database at the new path, creating it if needed. Data isn't moved (pre-alpha, D-30): pointing at an empty path starts with empty history. Requests already running finish on the old database.
- **Hub key (`identity_key_path`):** the hub uses the key at the new path (created if missing). Probes that fetched the old key by handshake must fetch the new one: set `hub_public_key` in their config (reloaded), or restart them. Automatic re-handshake belongs with enrollment (M2).
- **Admin token (`auth_token_path`):** the token file is read at each login, so a new path, or `--reset-admin-token` on a running hub, applies at the next login. Existing sessions stay valid.
- **Listeners (`host`, `port`, `[server.tls]`):** the hub stops accepting on the old address (in-flight requests get up to 5 s to finish) and starts on the new one. A new address that can't be bound is caught while preparing, so the reload is refused. If binding still fails at the switch (a race), the hub goes back to the previous address and logs an error. TLS certificate files are read (or created) when the listener starts. A renewed certificate is picked up at the next listener change, and watching certificate files is a follow-up.
- **Probe (`hub_url`, `hub_public_key`):** from the next cycle the probe uses the new hub URL and key. Setting `hub_public_key` pins the key; removing it makes the probe handshake with the (possibly new) URL. If the new URL or key is unusable (e.g. `https://` without an embedded certificate, or a malformed key), the probe keeps the old connection and logs why. Reports buffered for the old hub key are rejected by the hub (401) and dropped by the outbox (D-29); nothing else needs cleaning up.

## D-39 · Archiving stays; old data can be purged
**Status:** Accepted (2026-09-28, owner) · **Supersedes:** D-20 · **Settles:** D-26

- **Archiving stays** as a feature, whatever rollups (M6) bring later. So the archive viewer must work: D-26 is a bug to fix (M0-19), not a known limitation.
- **Old data can be deleted on purpose (new: M1-10).** Besides archiving (which moves data out), the hub gets a **purge**: delete data older than a period (days, weeks, months, years) or before a date, from the live database or from an archive, and delete a whole archive. Today the only deletions are "everything of a company/probe" (admin page).
- **Purge is permission-gated and audited**, so it builds on M1-01 (roles) and M1-06 (audit log): it is irreversible, so it asks for confirmation, shows what will be deleted first, and records who did it.

## D-40 · Probe self-update is important: phase 1 moves to M2
**Status:** Accepted (2026-09-28, owner) · **Design:** [0012](proposals/0012-probe-self-update.md) · **Supersedes:** the self-update sketch in 0004 §5.4

Probes must be able to detect a new version, download it from a URL and update themselves (owner request, marked important). The standalone part (signed manifest, download, self-test, switch, rollback) needs neither the command channel (0003) nor CI, so it moves from M4 to **M2 at P0**. Hub-managed and staged rollouts stay in M4. Non-negotiable: a probe installs only binaries signed with the release key compiled into it, never downgrades on its own, and rolls back a version that can't start or report.

## D-41 · Updates come from the hub; the signing key never lives on the hub
**Status:** Accepted (2026-09-28, owner) · **Refines:** D-40 · **Design:** [0012](proposals/0012-probe-self-update.md)

- **Probes update only from their hub,** not from an internet release server. This keeps them in sync with the hub's version and protocol, and client networks need no firewall opening beyond the hub (FP-1).
- **Whoever builds, signs.** Today the admin builds hub and probes, so the admin signs releases with their own key (`blentinelmake release sign`); the public key is compiled into the probe like the hub certificate. Official Blentinel releases, when they exist, use the same format with the Blentinel key.
- **The private signing key never lives on the hub.** A hub that could sign would turn a hub compromise into a compromise of every client network. The hub only stores and serves releases that are already signed.
- **Off by default, switched at the hub:** no update is offered until the admin approves a version. Probes keep a local emergency brake (`[update] enabled = false`).
- **The hub is never updated unattended.** Opt-in "new version available" notice, and an admin-triggered `hub upgrade` with a database backup and rollback (M4-04, after M2-07 and M2-08).
