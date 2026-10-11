# Blentinel — Decision Log & Challenged Assumptions

This file records design decisions and **challenges to earlier specs, docs, and assumptions**. Each entry has a status:

- **Proposed:** a recommendation that needs the project owner's approval.
- **Accepted:** agreed; the roadmap reflects it.
- **Rejected:** considered and declined, with the reason recorded.

Newest entries at the bottom. Never delete an entry. Supersede it with a new one.

---

## D-01 · Hub TLS certificate embedded in the probe at compile time → pin at enrollment
**Status:** **Done** (2026-09-28, M2-01), refined by D-47: the probe pins the SHA-256 of the whole certificate, not of its public key; rotation is still M2-13 · **Affects:** README "HTTPS Configuration", `probe/build.rs` (removed), HTTPS_SETUP_GUIDE.md · **Roadmap:** M2

**Today:** `probe/hub_cert.pem` is compiled into the probe binary. Every hub needs its own probe build, and a cert change means rebuilding and redeploying every probe.

**Challenge:** this contradicts PP-1 (trivial install) and makes renewing the certificate painful. It doesn't add security over pinning at runtime, because both approaches pin a single fingerprint.

**Decision:** the probe pins the **SHA-256 fingerprint of the hub's TLS public key**, which it receives in the enrollment code (proposal 0002). Everyone uses the same official probe binary. Certificate rotation: the hub announces the *next* fingerprint in report responses before switching, so probes accept both during an overlap window. FP-3 is preserved.

## D-02 · Unauthenticated `/api/handshake` key fetch → hub key comes from the enrollment code
**Status:** **Done** (2026-09-28, M2-01), more strictly than decided: the code carries the whole hub key, so `/api/handshake` and `agent.hub_public_key` are gone (D-47) · **Affects:** `probe/src/transport.rs::fetch_hub_pk`, `agent.hub_public_key` · **Roadmap:** M2

**Today:** if `hub_public_key` is not set, the probe fetches the hub's X25519 key over `/api/handshake` and trusts whatever it gets back (trust on first use, with no verification). An attacker on the network path at first contact could impersonate the hub.

**Decision:** the enrollment code carries the hub key fingerprint. The probe verifies the handshake result against it and refuses to proceed on mismatch.

## D-03 · HTTP as the default transport → HTTPS by default for the web UI
**Status:** **Done** (2026-09-28, M1-05) in split mode, see D-43 · **Roadmap:** M1

**Today:** HTTP is the default. Probe payloads are still protected because they are encrypted at the application layer, but the **web UI is not**.

**Challenge:** once usernames and passwords exist (proposal 0001), plain HTTP exposes credentials and session cookies.

**Decision:** the hub serves **HTTPS by default** with an auto-generated self-signed cert, optional ACME/Let's Encrypt, or documented reverse-proxy mode. Plain HTTP stays available only as an explicit opt-in (e.g. behind a TLS-terminating proxy). Session cookies are marked `Secure`.

## D-04 · Probe whitelist and alert settings in `blentinel_hub.toml` → database + UI
**Status:** Done: probes, companies and sites are in the database and managed in the UI (M1-03, M2-01; `[[probes]]` removed, not imported: D-30, D-47); alert channels and the mail server too (M3-03, D-66; `[alerts]` removed, not imported). The "import once" below is superseded by D-30 · **Affects:** `[[probes]]`, `[alerts]`, hub hot-reload · **Roadmap:** M1/M2

**Challenge:** editing TOML on the server to add a probe or technician violates PP-2.

**Decision:** probes, companies, sites, technicians and users, alert channels, and thresholds move to the database and are managed in the UI. The TOML keeps **bootstrap only**: bind address and ports, data directory, TLS file paths. On first start after the upgrade, existing `[[probes]]` and `[alerts]` entries are **imported once**. After that the sections are ignored with a warning. Hub config hot-reload shrinks accordingly (possibly to nothing).

## D-05 · Probe-local `[[resources]]` as the source of truth → hub-managed configuration
**Status:** Accepted (implemented in M2-04, see D-49: the probe file goes entirely) · **Affects:** `blentinel_probe.toml`, probe hot-reload · **Roadmap:** M2

**Challenge:** changing a check today means remote access to the client's machine, which violates PP-2.

**Decision:** checks are defined in the hub and delivered to the probe **in the response to its report** (FP-1 compliant; proposal 0003). The probe caches the last good config on disk (FP-4). The local TOML shrinks to bootstrap: hub URL(s) and data directory. **No local check overrides**: one source of truth. A `--standalone <file>` developer mode may remain for testing.

## D-06 · Shared admin token → per-user accounts
**Status:** **Done** (2026-09-28, M1-01 + M1-02, see D-42) · **Roadmap:** M1 · proposal 0001

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
**Status:** Accepted. README corrected (M0-12, 2026-09-27); web assets embedded in the hub binary in M2-07 (D-51) · **Affects:** README "Cross-Compilation" · **Roadmap:** M2

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
**Status:** Accepted (implemented in M2-04, see D-49) · **Roadmap:** M2 (with 0003)

**Challenge:** identity built from mutable text means renaming a check or changing its target loses its history, silences, and alert state, and a `:` inside a name breaks parsing.

**Decision:** checks become hub records with a UUID `CheckId` (proposal 0003). Results, silences, incidents, and alert state reference `CheckId`.

## D-16 · Binary Up/Down status → Up / Warning / Down / Unknown
**Status:** Accepted (implemented in M3-01, 2026-10-06: see D-64) · **Roadmap:** M3 · proposal 0005

**Today:** threshold breaches (disk ≥ 90%, etc.) only trigger an e-mail. The UI still shows the resource as **Up**, and a probe's stale devices are also shown as their last state.

**Decision:** four states, evaluated **once, on the hub** (a single source of truth). *Warning* covers threshold breaches. *Unknown* covers stale data or an unreachable parent dependency.

## D-17 · Probe-reported `company_id` is trusted → hub assigns company and site
**Status:** **Done** (2026-09-28, M2-01): the report's `company_id` and `site` are ignored, not deprecated first (D-30, D-47) · **Roadmap:** M2 · proposal 0002

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
**Status:** **Fixed** (M0-19, 2026-09-28), see *Resolution* · **Found by:** M0-04 characterisation tests · **Roadmap:** M0-19

**Found:**

- Archiving prunes `probe_heartbeats` rows whose `last_seen_at` is after the cutoff. The archive dashboard groups by heartbeat, so any probe **still reporting today disappears from the archive**, even though its old reports are in it.
- Archive uptime asks for "the last 24 h relative to now" (and ignores the `range` parameter), but an archive only holds data older than its cutoff, so it is **always empty**.

**Evidence:** `hub/tests/admin.rs` → `known_issue_archive_shows_companies_whose_probes_are_still_reporting`, `known_issue_archive_uptime_contains_archived_data`.

**Decision (proposed):** because D-20 may retire the archive viewer once rollups exist, fix cheaply now only if archives are used in practice (P2): keep heartbeats in the archive and compute uptime relative to the archive's cutoff date. Otherwise record it as a known limitation until D-20 is decided.

**Resolution (M0-19, 2026-09-28; D-39 decided to keep archiving):**

- The archive keeps the heartbeat of every probe it has reports from. For a probe seen after the cutoff, `last_seen_at` is set to its last archived report (and `first_seen_at` capped at that), so the archive doesn't claim the probe was seen "today". Probes with no archived reports are removed as before.
- **Challenge to the proposed "relative to the archive's cutoff":** uptime is anchored at the archive's **newest report** (`ReportStore::latest_report_at`), not the cutoff. The two are the same when probes report up to the cutoff, but if reporting stopped well before it, a cutoff anchor still gives empty 24h/7d/30d charts. The newest report is never later than the cutoff, and needs no new storage (the archive file doesn't record its own cutoff).
- The archive uptime endpoint honours `?range=`. The UI gains a read-only archive company page (`/archive/{id}/company/{company}`, the live page with polling and silencing turned off), so the range buttons exist in an archive at all.
- The archive boundary is now exact: the archive keeps reports `< cutoff`, the main database keeps `>= cutoff` (a report exactly at the cutoff used to be in both).

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
- **Hub, Windows:** a scheduled task (at boot, SYSTEM, restart on failure) instead of a broken service. Program in `C:\Program Files\Blentinel\Hub`, config and data in `C:\ProgramData\Blentinel\Hub`. A real Windows-service mode is still desirable (M2-08). *(Done in M2-08, D-52: `hub install` registers the Windows service `BlentinelHub`; the hub's installer scripts are gone.)*
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
**Status:** Accepted (2026-09-27, owner request); the admin-token bullet is superseded by D-42 (no token any more) · **Supersedes:** the "needs a restart" list in `reference/configuration.md`

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
**Status:** Accepted (2026-09-28, owner); phase 1 done (2026-10-04, M2-15, D-55) · **Design:** [0012](proposals/0012-probe-self-update.md) · **Supersedes:** the self-update sketch in 0004 §5.4

Probes must be able to detect a new version, download it from a URL and update themselves (owner request, marked important). The standalone part (signed manifest, download, self-test, switch, rollback) needs neither the command channel (0003) nor CI, so it moves from M4 to **M2 at P0**. Hub-managed and staged rollouts stay in M4. Non-negotiable: a probe installs only binaries signed with the release key compiled into it, never downgrades on its own, and rolls back a version that can't start or report.

## D-41 · Updates come from the hub; the signing key never lives on the hub
**Status:** Accepted (2026-09-28, owner); built for probes in M2-15 (D-55: the brake is `probe updates off`, approval is set in the UI); hub upgrades built in M4-04 (D-75: `hub upgrade` on the server) · **Refines:** D-40 · **Design:** [0012](proposals/0012-probe-self-update.md)

- **Probes update only from their hub,** not from an internet release server. This keeps them in sync with the hub's version and protocol, and client networks need no firewall opening beyond the hub (FP-1).
- **Whoever builds, signs.** Today the admin builds hub and probes, so the admin signs releases with their own key (`blentinelmake release sign`); the public key is compiled into the probe like the hub certificate. Official Blentinel releases, when they exist, use the same format with the Blentinel key.
- **The private signing key never lives on the hub.** A hub that could sign would turn a hub compromise into a compromise of every client network. The hub only stores and serves releases that are already signed.
- **Off by default, switched at the hub:** no update is offered until the admin approves a version. Probes keep a local emergency brake (`[update] enabled = false`).
- **The hub is never updated unattended.** Opt-in "new version available" notice, and an admin-triggered `hub upgrade` with a database backup and rollback (M4-04, after M2-07 and M2-08).

## D-42 · User accounts: how M1-01 and M1-02 were built
**Status:** Accepted (implemented 2026-09-28, M1-01 + M1-02 together, owner's choice) · **Implements:** D-06, proposal 0001 §4.1–4.4 · **Supersedes:** the admin-token parts of D-38

The owner chose to ship users (M1-01) and the setup wizard, CLI and token retirement (M1-02) as one change, so the hub is never in a state with users but no way to create the first one. Choices made, and where they differ from proposal 0001:

- **Access is declared per route, in one table.** Each route in `hub/src/routes.rs` states its `Access` (`Public`, `Probe`, `LoggedIn`, or `Perm(permission)`), and the router applies one middleware from it. A route can't be registered without an access level, and `hub/tests/access.rs` walks the same table with every role, which is the "matrix generated from the router" proposal 0001 §5 asked for. Proposal 0001 sketched an extractor per handler; a table can't be forgotten in a new handler.
- **One rule, `model::authorize(role, permission)`,** shared by the server and the browser (which only hides what it can't do). Permissions today: `ViewData`, `Silence`, `ViewAdmin`, `ManageData`, `ManageUsers`. Finer permissions (`EditChecks`, …) arrive with the features that need them. "Only Owners manage Owners", the last-Owner guard and "not yourself" are rules about the *target* user, so they live in `service::users`, not in the permission table.
- **Users limited to some companies are refused until M1-04.** The schema stores `all_companies` and `user_companies`, but nothing filters by them yet. Letting someone create a Client or a limited Viewer now would give access that only *looks* limited. So `client`, and `all_companies = false`, answer 400 `not_supported` until scoping is enforced.
- **The setup code is also written to `setup.code`** (owner-only, deleted once used), besides the log. The Windows hub runs as a SYSTEM scheduled task whose output is hard to reach, and a file is easier to `cat` than a journal. A hub restart before setup makes a new code. Because users live in the database, switching `db_path` to an empty database also creates a code.
- **`hub user` prints temporary passwords** instead of prompting for one: no terminal/no-echo dependency, and the same "must change at next login" path as the web UI's resets. `hub --reset-admin-token`, `auth_token_path` and `hub_auth.token` are gone (pre-alpha, D-30). An old config that still sets `auth_token_path` loads; the key is ignored.
- **Sessions:** 12 h idle, 7 days absolute, as constants (a setting when someone needs one). Only SHA-256 of the cookie is stored; sessions survive restarts; an hourly task deletes dead ones. `SameSite=Strict` now; `Secure`, the CSRF header and rate limiting stay in M1-05 with HTTPS by default.
- **Archives never contain users or sessions:** archiving copies the whole database file, so the archive copy empties those tables before its VACUUM.
- **argon2 is compiled optimised even in debug builds** (`[profile.dev.package.argon2]`), or every test login would take seconds.

## D-43 · HTTPS by default in split mode; CSRF header; login rate limiting (M1-05)
**Status:** Accepted (implemented 2026-09-28; split mode chosen by the owner) · **Implements:** D-03 · **Related:** D-01

**Problem:** D-03 says "HTTPS by default", but probes can only use HTTPS when built with the hub's certificate (until enrollment pinning, D-01, M2). Making the main port HTTPS-only would cut off every existing probe until rebuilt.

**Decision (owner):** **split mode** is the default. HTTPS on `https_port` (3443) serves everything. Plain HTTP on `port` (3000) serves **only** the probe endpoints, whose reports are signed and encrypted end to end, and answers everything else with a 308 redirect to the same path on HTTPS. Passwords and session cookies never cross plain HTTP, and probes keep working unchanged. Details:

- `[server.tls] enabled` defaults to `true` and `https_port` to 3443, also when the section is missing. `https_port == port` means HTTPS only; `enabled = false` is the explicit opt-out (plain HTTP for everything). `https_port` is no longer optional: "absent" can't mean both "default 3443" and "HTTPS only".
- The probe paths for the HTTP port come from the route table (`Access::Probe`), not a second list. Each listener marks its requests (`auth::Transport`), and handlers read `auth::RequestInfo` (secure or not, client address).
- **`Secure` cookies** when the request came over HTTPS, or with **`[server] behind_proxy = true`** for a TLS-terminating proxy. That flag also takes the client address from the last `X-Forwarded-For` hop, so it trusts the proxy and is documented as never for directly reachable hubs.
- **CSRF:** every POST except probe endpoints needs `X-Blentinel: 1` (403 `csrf`), including login and setup (login CSRF). It complements `SameSite=Strict`: a cross-site form can't set the header, and a cross-site `fetch` with it needs a CORS preflight the hub never grants. Scripts must send it (documented).
- **Rate limiting** (in memory, reset by a restart; locks are short): per (client IP, username), 5 free failures, then 30 s doubling to 15 min, cleared by a success; per IP, 20 failures in an hour lock the IP for 15 min (password spraying). A locked attempt is refused with 429 + `Retry-After` **before** the password is checked, so locking can't be probed for timing. Wrong setup codes use the same limiter (username `#setup`). A persistent or shared limiter isn't needed at the target scale (one hub).
- **Browsers warn** about the self-signed certificate; the docs explain checking the fingerprint against the log. ACME/Let's Encrypt stays M2-14.

## D-44 · Companies and sites: registered by reports, managed in the UI (M1-03)
**Status:** Accepted (implemented 2026-09-28; the owner chose auto-registration and no timezone yet). **Auto-registration superseded by D-47** (M2-01): companies and sites are created in the UI, and probes are placed by enrollment · **Implements:** proposal 0002 §4.1 (in part) · **Related:** D-17, M2-01

- **Auto-register, then manage.** A report naming an unknown company or site creates the record (marked *new*: `reviewed_at` empty). Admins name it, add contacts, notes and addresses, or archive it. Reporting never stops for an unknown name. The alternatives, rejecting reports for unknown companies or having the hub assign probes now, were declined: the first adds a step before any probe can report, the second is M2-01 (enrollment) and D-17.
- **The keys are the text probes send** (`companies.id` = `company_id`, `sites (company_id, id)` = `site`), not new random IDs as proposal 0002 sketched. Reports, heartbeats, alert keys and user scopes (M1-04) keep using them, so no data moves. They are immutable; display names are separate. When M2-01 makes the hub assign tenants, the records stay and probes point at them.
- **No site timezone yet:** nothing uses it before maintenance windows (M3-04), which add it with a migration.
- **Archiving hides, it doesn't block:** an archived company leaves the dashboard, but its probes still report and alert, and a new report doesn't unarchive it. Blocking a tenant belongs to probe revocation (M2).
- **Probe-sent names can't be merged** (e.g. a typo): fix the probe's config and archive the typo. Moving history between companies can come with M2's probe moves if needed.
- Contacts and notes are internal; M1-04 decides what Client users may see.

## D-45 · Which companies each user sees (M1-04)
**Status:** Accepted (implemented 2026-09-28; scope modes, Client view and Technician rights chosen by the owner) · **Implements:** proposal 0001 §4.1 (company scoping)

The hub serves anything from one person monitoring their own company (one Owner, nothing to configure) to an IT provider with technicians, a boss and customer logins. The owner's example: Tech A is admin; Tech B has customers 1–5; Tech C has 4, 5, 7, 8; the boss views all except 8; customers 2, 4, 7 see their own company; customer 3 sees 3 and 5.

- **Three scope modes:** all companies (the default), only these, all except these. "All except" includes companies registered later, which matters because companies register themselves (D-44). Stored as `users.company_scope` + `user_companies`. Owner and Admin always see all (`UserInfo::effective_scope`), whatever is stored.
- **Lists are filtered in the storage layer** (`CompanyScope` passed to `dashboard_companies` and `companies`), so no list can leak. Single items are checked by `service::scope` before reading (company, probe via `probe_company`, silence via its resource key, archive routes).
- **Outside the scope is 404, not 403:** a customer can't discover other customers.
- **Clients** (owner): everything about their own companies (dashboard, probes, devices, uptime, archives), read-only, but without internal notes and contact fields (`without_internal_fields`). They must have "only these".
- **Technicians** (owner): may edit company and site details within their scope (`ManageCompanies` is no longer Admin-only). Archiving hides a company from everyone, so it moved to `ManageData` (Admin). Later features (checks, maintenance) follow the same rule: Technicians manage them in their companies.
- **Guard:** `hub/tests/scoping.rs` lists every route open to scoped users; a new one fails the build until it has a scoping test or is declared not to carry company data.
- `hub user create` stays all-companies (break-glass); Clients are created in the UI.

## D-46 · The audit log (M1-06)
**Status:** Accepted (implemented 2026-09-28) · **Implements:** proposal 0001 §4.2 (`audit_log`), §4.5 (audit UI)

- **Recorded where the actor is known:** the HTTP handlers (user + client IP from `RequestInfo`), the `hub user` console, and ingestion (companies and sites registered by reports, recorded as the hub itself). Actions are named `group.verb` (`service::audit::action`), so the filter can match a group by prefix.
- **What:** logins (success, failure with the username tried, logout), setup, own password and sessions, user management, company/site edits, archiving, silences, data deletion, probe removal, archive creation. Not recorded: reads, and the hub's automatic archiving (not a person's action; it is in the hub log). Details keep the values set, never passwords, temporary passwords or tokens.
- **The username is stored with the entry,** as it was, so entries stay readable after a user is renamed or removed.
- **Best effort:** a write failure is logged but never fails or undoes the action, so a full disk can't lock admins out. The proposal wrote failed logins to the audit log for rate limiting; the limiter stays in memory (D-43), and the log is for people.
- **Append-only and Admin-only** (`ViewAdmin`). It isn't scoped per company yet; a later item can show Technicians the entries of their companies (entries carry `company_id` for that).
- **Not in archives:** archives hold monitoring history only (like users and sessions, D-42). Deleting old entries belongs to data purge (M1-10).

## D-47 · Probe enrollment: codes, records, pinning, the hub assigns the tenant (M2-01)
**Status:** Accepted (implemented 2026-09-28; the owner chose removing `[[probes]]`, the hub assigning company and site at "Add probe", and single-use 24 h codes by default; the default use limit is superseded by D-54) · **Implements:** proposal 0002 §4.2–4.4, D-01, D-02, D-17 · **Supersedes:** the `[[probes]]` whitelist (D-04, for probes), auto-registration (D-44)

- **The code is self-contained:** `BLT1-` + Crockford base32 of version, hub URL, the **whole** X25519 hub key, the certificate fingerprint (optional), token ID, secret, CRC-32 (`common::enrollment`, format in `reference/protocol.md`). About 200 characters, for copy and paste. With the whole key in the code there's nothing to fetch, so `/api/handshake` is removed rather than verified (D-02).
- **Pin the certificate's SHA-256, not the SPKI** (refines D-01): it is the fingerprint the hub logs and browsers show, so an admin can compare them, and the probe needs no X.509 parser. The cost is the same as for SPKI while rotation doesn't exist (a new certificate means new codes); M2-13 can switch to announcing the next fingerprint. The TLS handshake signature is still verified.
- **Pin only when it applies:** the hub adds its fingerprint only for an `https://` URL and only if it serves TLS itself. Behind a proxy with a public certificate the probe uses the system roots; an `http://` URL stays plain HTTP (the payload is still sealed and signed). The UI pre-fills the URL from the browser's address, which is the HTTPS port.
- **Enrollment is sealed and signed like a report:** it proves the probe holds the key it registers, and hides the token secret even over HTTP. Only a SHA-256 hash of the secret is stored; redemption is one atomic `UPDATE … RETURNING`, so a single-use code can't be used twice concurrently. Failed codes use the login limiter (key `#enroll`).
- **Probe records** (`probes`) decide company and site; the report's `company_id`/`site` are overwritten at ingest (not deprecated first: D-30), so storage, heartbeats and alert keys keep their shape. `probe_heartbeats` stays the liveness table; merging it into `probes` waits until something needs it.
- **Re-enrollment replaces the record** (company, site, name; active again) instead of answering 409: moving a probe to another hub, company or site is "enroll again", the same gesture everywhere. Holding a valid code for the target company is the authority.
- **Who may do what:** add sites, create and revoke codes, rename and move probes: `ManageCompanies`, within the scope (Technicians in their companies). Create companies: `ManageData` (Admin), since a scoped Technician's scope can't name a company that doesn't exist yet. Revoke a probe: `ManageData` (like removing one); a revoked probe gets `403 probe_revoked` and must enroll again.
- **Companies and sites are created by hand** (Add company, Add site); reports no longer create them. With enrollment the report's text is no longer trusted, so it can't name records either.
- **Probe config shrinks** to `interval` + `[[resources]]`; the hub, company and site live in `enrollment.json`, written atomically by `probe enroll` and followed by a running probe each cycle. A probe without it waits instead of exiting, so a service can be installed before it's enrolled.
- **Not done here:** approval of enrollments (a later item), per-OS install one-liners (M2-09), rotation (M2-13).

## D-48 · The hub's answer to a report: signed, sealed, bound (M2-03)
**Status:** Accepted (implemented 2026-09-29) · **Implements:** proposal 0003 §4.2 (the envelope) · **Refines:** 0003's `ReportAck` sketch

- **Signing key derived from the identity** (`common::crypto::hub_signing_key`: SHA-256 of a label and the X25519 secret, used as the Ed25519 seed). One key file to protect, back up and configure, with no new setting. The cost is that both keys rotate together; M2-13 announces both in the same signed answer. The public half is in the enrollment code (code format 2), so the probe verifies it independently of the transport, including over plain HTTP.
- **Sealed to the probe's identity:** the ack is sealed like a report, to the X25519 form of the probe's Ed25519 key (`to_montgomery` / `to_scalar_bytes`), instead of reusing the report's ephemeral key. Reports are sealed before they're buffered, so that key is gone by the time a buffered report is delivered. The probe needs no second key. Using one key for signing and for key agreement through the birational map is an established construction (libsodium, age's ssh-ed25519 recipients).
- **Sign the bytes, not a re-serialization:** the wire form is `[signature][ack JSON]`, so a newer hub can add fields (config, commands, next keys) without breaking older probes' verification. The enrollment request, verified only by the hub, keeps its simpler form.
- **Bound by the report's SHA-256** (not its timestamp): the probe can check it from the sealed bytes it sent, including buffered ones, without opening its own report.
- **The HTTP status stays the outcome** (200, 409, 4xx, 5xx), so buffering rules, proxies and logs are unchanged. The proposal's `accepted` flag is dropped. Acks go only to probes whose signature verified. A revoked probe gets `403` **with** a signed ack (`revoked: true`); other errors keep the JSON error body.
- **Revocation must be authenticated to act on it:** only a signed revocation stops the probe (enrollment set aside, buffer cleared, wait for a new code). A plain 403 only drops the one report. If the file can't be moved, the probe keeps its normal interval rather than spinning.
- **An untrusted answer to a 200 is ignored, not fatal:** the report was stored, and the probe logs the warning; nothing in the ack is acted upon.
- **Protocol v3,** accepting v2 (current and previous, ENGINEERING §2.7): reports are unchanged, and a v2 probe ignores the body. v1 (no signed version) is dropped. M2-15's `probe_version` moves to the next version.
- **Only what's used now:** `report_sha256`, `hub_time` (clock-skew warning at 60 s), `revoked`. Config, commands and rotation fields arrive with M2-04, M2-06 and M2-13.

## D-49 · Checks as hub records, pushed in the report answer (M2-04)
**Status:** Accepted (implemented 2026-09-30) · **Implements:** proposal 0003 §4.1–4.2 (checks and config), 0006 §1 rules 2–4, D-05, D-15 · **Refines:** 0003's `probe_config_state` and version counter, its import of `[[resources]]`, and 0006's `timeout` setting

- **The probe has no config file.** Checks *and* the report interval are hub records; the interval moves to the probe record (`probes.interval_secs`, 5–3600 s, set with `POST /api/probes/{id}`). `blentinel_probe.toml`, its hot reload and `probe --init` go (`--init` moves to the hub's own flags). The probe's state is `identity.key`, `enrollment.json`, `config.json` and its buffer. D-05 said the TOML "shrinks to bootstrap"; the bootstrap (hub URL, company, site) already moved to `enrollment.json` (D-47), so nothing is left.
- **The config revision is a content digest, not a counter:** the first 16 hex digits of a SHA-256 over the config's JSON (enabled checks by ID, interval). No `desired_version`/`applied_version` columns to keep in step, no collision after a hub database is recreated, and "in sync" is a string comparison. The probe echoes the revision the hub sent (it never recomputes it), so a newer hub may add fields.
- **Sent when the revisions differ, not when the hub's is greater.** A probe moved to another hub, or a hub reinstalled, converges either way. The one exception: a revision the probe refused isn't sent again (it would be refused again); any change makes a new revision, which is.
- **The config rides in the signed ack; no second signature.** The ack is already signed by the hub's pinned key, sealed for the probe and bound to its report (D-48). `config.json` is a cache in the probe's own folder, like `identity.key`; whoever can write it can also replace the identity, so signing the file would add nothing.
- **All or nothing on the probe.** `ProbeConfig::validate` (the same code as the hub's API) runs on every config received; a config that fails, or doesn't parse (a kind this probe version doesn't know), is refused whole, reported as `config_rejected {revision, error}` and shown by the hub as `sync: rejected`. Per-check degradation can come with self-update (M2-15) if version skew makes it worth it.
- **A new config runs at once.** The probe starts the next cycle immediately instead of an interval later, so a change reaches results within one interval.
- **A probe reports only on its own checks.** Ingest keeps results whose `check_id` is one of the probe's checks (and drops repeats), since alert state and silences are keyed by check ID: otherwise one probe could raise or clear another's alerts (FP-3).
- **Results keep history, checks don't own it.** `check_results.check_id` has no foreign key: deleting a check keeps its past results in uptime history, but the device list shows only existing checks. Deleting a check deletes its silence. A probe's checks are deleted with its record, and survive re-enrollment (like its interval).
- **Silences and alert state are keyed by check ID** (`scope_type = 'check'`); the silence API takes `check_id` and scope is checked through the check's probe. `company:probe:name:target` keys (`model::resource_key`) are gone.
- **Several metrics per result** (`check_metrics`, ordered; the first is the headline). Thresholds apply to the `usage` metric of `cpu`, `memory` and `disk`.
- **Kinds renamed** (`local_cpu` → `cpu`, …); `local_data` dropped (it expanded into four checks the hub can now list itself). Validation is what the probe can do today (e.g. `ping` needs an IP address); richer settings (timeouts per check, HTTP options, hostnames for ping) come with the check catalogue (M5, proposal 0006). Every check has a 30 s limit on the probe.
- **Protocol v4 only** (MIN = 4), against ENGINEERING §2.7's "current and previous": a v3 report has no check IDs to file results under. Pre-alpha, no compatibility (D-30). The hub now reads `protocol_version` before parsing the rest, so an old probe gets `unsupported_protocol_version`, not `not_a_report`. The unused `ephemeral_public_key` field is gone.
- **No `[[resources]]` import** (proposal 0003 §4.4 and the roadmap row): pre-alpha data is disposable (D-30). Until the Checks page (M2-05), checks are managed through the JSON API.

## D-50 · The probe page for checks (M2-05)
**Status:** Accepted (implemented 2026-09-30) · **Implements:** proposal 0003 §4.5 · **Refines:** its "Checks tab" and bulk actions

- **A page per probe** (`/probes/{id}`), not a tab on the company page: a probe's checks, results, interval and sync state are one thing to look at, and the page has a stable address to link to. Probe names on both company pages link to it.
- **The form validates in the browser with `common::checks`**, the code the hub and the probe run, so the user sees the reason before anything is sent; the hub checks again. The form's field mapping is plain functions (`CheckFields`), unit-tested without a browser.
- **"Copy checks from another probe" is in**, from 0003's bulk actions: it is what setting up a second, similar machine needs. Copies get new IDs (their own history and alerts). Both probes must be in the user's companies. Templates (named sets applied to many probes) stay later (0006 §5).
- **Not in this item:** "run now" needs the command channel (M2-06); editing every setting at once across probes needs templates. Delete has no confirmation, like the page's other actions; a check's history stays anyway (D-49).

## D-51 · The web UI embedded in the hub binary (M2-07)
**Status:** Accepted (implemented 2026-09-30) · **Implements:** D-11, proposal 0004 §4.1 (one binary)

- **A `build.rs`, not a crate** (`rust-embed`, `include_dir`): about 80 lines generate `include_bytes!` entries with an ETag each; no new dependency, and nothing read from disk at runtime (`rust-embed` reads from disk in debug builds, which would hide a broken embed from the browser tests).
- **Opt-in with `BLENTINEL_EMBED_SITE=<site root>`, in a second build step.** cargo-leptos 0.3 empties `target/site` and then builds the front end and the server **in parallel** (sequentially only with hashed file names or precompression), so a server build script reading `target/site` could embed a missing or half-written WASM file. `blentinelmake hub build` and the browser-test build run `cargo leptos build --frontend-only`, then `--server-only` with the variable; `build.rs` refuses a folder with no `.wasm`. Hashed file names would make cargo-leptos build in order, but Leptos then reads the hash file from beside the binary at runtime, which is a second file again.
- **Not embedded otherwise, served from disk:** `cargo leptos watch` keeps live CSS reload (the server isn't rebuilt for a style change), and `cargo test-all` doesn't compile 25 MB of debug WASM into every test binary. The hub logs which it uses. The old lookup of `pkg/` beside the executable is gone: a published hub always embeds.
- **Caching:** file names aren't content-hashed, so the files are served `Cache-Control: no-cache` with a strong ETag (64-bit FNV-1a of the content); a browser revalidates and gets `304` until the hub is upgraded.
- **The browser tests prove it:** the e2e hub runs from a folder holding only its config, where Leptos's `site-root` (`target/site`) doesn't exist.
- **Not in this item:** release targets (musl, ARM64) and the Docker image are CI (on hold, D-24) and M2-11.

## D-52 · The hub installs, checks, backs up and removes itself (M2-08)
**Status:** Accepted (implemented 2026-09-30) · **Implements:** proposal 0004 §4.2, the Windows ACL item of security.md, D-34's "a real Windows service" · **Refines:** 0004's `uninstall --keep-data`, `doctor`'s clock check

- **`hub install` replaces the installer scripts.** The binary copies itself into place (staged beside the target, then renamed, so a running old binary is never half-written), writes the config on a first install from the `--init` template (`config::install_config`: one source), secures the data folder, and registers the service. Re-running it is the upgrade: stop, replace, start; config and data are kept, and options for where to listen are ignored with a note once a config exists. The package is now the binary and its checksum.
- **A first install listens on all interfaces** (`0.0.0.0`, ports 3000 and 3443). The template's `127.0.0.1` suits a trial by hand, but an installed hub exists for probes on other machines to reach; the old scripts made every admin edit the file after installing.
- **The hub may use system tools** (`systemctl`, `useradd`, `chown`): FP-2 binds the probe only. Windows uses the Service Control Manager and the Win32 security API instead of `sc.exe`/`icacls`, so M2-09 can reuse that code in the probe, which must not run external tools.
- **Windows: a real service** (`BlentinelHub`, LocalSystem, automatic, restart after 5 s up to three times a day) instead of the scheduled task. `main` builds its own Tokio runtime so the SCM dispatcher can take the main thread first (as the probe does). A service has no console: the log goes to `hub.log` beside the config. Stopping ends the process; SQLite in WAL mode is consistent after that.
- **Windows data folder: `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`**, set with `SetNamedSecurityInfoW` (protected, inherited by everything inside); `hub doctor` reads it back and warns otherwise. Linux: `/var/lib/blentinel` `0700 blentinel`, the config `0640 root:blentinel` (it may hold the SMTP password); the unit adds `CAP_NET_BIND_SERVICE` so the hub can use port 443 without root.
- **`uninstall` keeps the config and data unless `--delete-data`**, the opposite of 0004's `--keep-data`: losing a hub's database (and its key, which every probe trusts) to a mistyped command costs far more than a leftover folder. Windows can't delete the running `hub.exe`, so it says to delete the program folder afterwards.
- **Default config:** `--config`, else `blentinel_hub.toml` in the working directory, else the installed hub's. So `hub status` works from anywhere on the server, and a hub run by hand in a folder keeps using that folder's config.
- **`hub status` and `hub doctor` only read:** the database is opened read-only, without migrating. `doctor` checks config, service, data folder (exists, writable, private), key, database (opens, `quick_check`, schema not newer than this hub's), certificate (expired → fail, < 30 days → warn), ports (answering with the service up; free without it), and disk space (< 500 MB fail, < 2 GB warn). No clock check: without a reference (NTP) it has nothing to compare to; probes already warn about skew against the hub (D-48).
- **Backup = one `.tar.gz`**: `manifest.json` (format, hub version, time, schema version), the database copied with `VACUUM INTO` (consistent while the hub writes, behind `ArchiveStore::backup_to` so another backend can do its own), the identity key, TLS certificate and key, and the config. Written owner-only (`common::fs::create_secret_file`) because it holds the private key. `tar` + `flate2` are pure Rust.
- **Restore puts files where the current config says** (a new server has its own paths), refuses while the hub runs or when the backup's schema is newer than this hub's, reads only fixed entry names (the archive can't choose where files go), moves what it replaces (with the WAL files and `.pub`) to `before-restore-<time>/`, writes a `.pub` for the restored key, and keeps the backed-up config as `<config>.from-backup` instead of overwriting the current one.
- **Verified** on a systemd host (the `blentinel-systemd-test` container): non-root refused; install, the web UI served, `status`, `doctor` all OK; backup while running, restore refused while running, restore after stopping, `doctor` OK after; upgrade by re-running install (key and config kept); uninstall with and without `--delete-data`. On Windows, unelevated: the folder permissions applied and read back (test), `install` refused, `status` and `doctor` on a hub run by hand. The owner then ran `hub.exe install` elevated on Windows (2026-09-30): the service installed and the hub was set up through it.

## D-53 · The probe installs itself; the hub serves it (M2-09)
**Status:** Accepted (implemented 2026-09-30); downloads come from signed releases since D-55 (the `probes/` folder is gone) · **Implements:** proposal 0004 §5.1–5.3, D-13, the probe folder ACL item of security.md · **Refines:** 0004's `/opt/blentinel-probe` layout, its pinned-curl one-liners, `uninstall --keep-data`

- **`probe install` replaces the installer scripts**, as `hub install` did (D-52): it copies itself into its folder (staged, then renamed), makes the folder private, enrolls with `--code` if given, registers and starts the service. Running it again is the upgrade (identity, enrollment, checks and buffer kept); a new code enrolls again in place.
- **One folder holds everything** (FP-2): program, identity, enrollment, `config.json`, buffer, log. Linux `/var/lib/blentinel-probe`, Windows `C:\Program Files\Blentinel\Probe` (SYSTEM + Administrators, the ACL code of D-52). A probe run by hand still keeps its files beside itself. 0004's `/opt` + separate data folder would split what the probe treats as one directory.
- **Linux runs it as a systemd `DynamicUser`** with `StateDirectory=blentinel-probe` (mode `0700`) and only `CAP_NET_RAW` (ambient and bounding set): no user to create, a folder only the service reads, and ping without root or `setcap` (which a replaced binary loses). systemd moves the folder to `/var/lib/private/blentinel-probe` behind a link; `install` and `uninstall` follow it.
- **systemd over D-Bus, not `systemctl`**: FP-2 forbids the probe external tools, so it writes the unit file and calls the manager (`Reload`, `EnableUnitFiles`, `RestartUnit`, `StopUnit`, `DisableUnitFiles`) with `zbus` (pure Rust, Tokio). Only the system bus: zbus panics on systemd's private socket (`/run/systemd/private`, no unique names), so a machine without D-Bus gets a clear error and runs the probe by hand. Windows uses the SCM and the Win32 security API, now in **`common::service`** (feature `service`), shared by the hub and the probe.
- **The hub serves probe binaries from `<data folder>/probes/`** (`probe-<target>[.exe]`; four musl/msvc targets) with their SHA-256, cached by size and modification time. A release hub carries its probes (`blentinelmake hub publish --with-probes`); nothing is fetched from elsewhere. Routes: `GET /api/probe-downloads` (`ManageCompanies`) and `GET /download/probe/{target}`, open and served on the plain HTTP port like the probe endpoints: a new machine has no session, and Windows PowerShell 5.1 can't follow the redirect to HTTPS. `probes_only` matches `{param}` segments to allow it.
- **The one-liners trust the hash, not the transport.** 0004 proposed `curl --pinnedpubkey` fetching a served script piped to `sh`; PowerShell has no equivalent pin, and a script piped to a shell is harder to review. Instead the wizard puts the file's SHA-256 (read on the hub's HTTPS page the admin already trusts) in the command: `curl -k … && sha256sum -c && sudo ./probe install --code`; on Windows `$ErrorActionPreference='Stop'`, the hash computed with .NET (`Get-FileHash` failed to load in a 5.1 session started from PowerShell 7) and `& $f install` only inside `if ($h -eq '<sha>')`. Found while testing: with a failing hash statement, PowerShell 5.1 skipped it and ran the probe anyway; the fail-closed form was verified in 5.1 and 7 (right hash runs, wrong hash throws, no hub runs no stale file). The probe then pins the hub from the code, as enrollment always did.
- **`uninstall` keeps the identity unless `--delete-data`** (as for the hub, D-52): reinstalling keeps the same probe ID and its history. Windows can't delete the running `probe.exe`, so it says so.
- **`doctor` checks the clock against the hub's `Date` header** over the pinned connection (warn beyond 60 s, fail beyond the hub's 5-minute limit), and the hub's reachability without sending a report. `status` and `doctor` open the buffer read-only.
- **Not in this item:** `--quiet` and `--dir`/`--no-service` (0004 §5.3), `probe run --standalone`; a hub fetching probes from a release server (M4); signed binaries (M2-15).
- **Verified** in the `blentinel-systemd-test` container: the wizard's Linux line against a hub installed with probes, install, enroll, reports, a ping check Up as the dynamic user, status, doctor, upgrade keeping the identity, uninstall with and without `--delete-data`, and the error without D-Bus. On Windows: the one-liner's download and hash check in both PowerShells; an elevated `probe install` is left to the owner.

## D-54 · Enrollment codes without a use limit by default
**Status:** Accepted (2026-09-30) · **Supersedes:** D-47's "single-use by default"

- **Asking for a number of probes up front made admins guess**: one code too few meant a second code for the next machine, too many left a code open. The owner asked for no limit. A code is now valid for **any number of probes until it expires** (24 hours by default, at most 30 days) **or is revoked**; a limit (1–1000) stays available for the cautious or scripted case.
- **Why that's acceptable (zero-trust, FP-3):** a code only lets a machine join as a probe of one company and site. It reads nothing, and the hub still assigns the tenant. Every enrollment is audited and shows on the company's page, where the probe can be revoked. The expiry bounds how long a leaked code works, and Revoke ends it at once.
- **Storage:** `max_uses = 0` means no limit in the SQLite column (it is `NOT NULL`), kept inside the backend; the `Storage` trait and the API use `Option` (`None`/`null`). Redemption stays one atomic `UPDATE` (`max_uses = 0 OR uses < max_uses`).

## D-55 · Probe self-update, phase 1: how M2-15 was built
**Status:** Accepted (implemented 2026-10-04); "accepting v4" superseded by D-56 (protocol v6 accepts v5); its "not in phase 1" list built in D-74 (M4-01) · **Implements:** proposal 0012 §4–5, D-40, D-41 · **Refines:** 0012's polling of `/api/update/probe/`, its `[update]` probe settings, `[updates] probe_version` in the hub's TOML, `min_protocol`, `allow_downgrade`; D-53's `probes/` download folder

- **The offer rides in the signed answer to a report**, not a separate endpoint polled every 6 hours. The answer is already signed by the pinned hub key, sealed for the probe and bound to its report (D-48), and the probe gets one every interval, so the probe learns of an approved version within one interval and needs no timer, jitter or extra request. The hub offers the approved release to a probe that reports another version, unless that's the version the probe last refused (D-49's rule for configs). The offer carries `manifest.json` and its signature as stored; the probe then downloads only its binary (`GET /download/releases/{version}/{file}`, a probe route on both ports, like D-53's downloads).
- **Trust comes from the release signature, not the answer's.** The probe installs a binary only if `manifest.json` is signed by a release key compiled into it (`BLENTINEL_RELEASE_KEYS` at build time, set by `blentinelmake` from every `.pub` in its key folder), the version is newer than its own, and the binary's SHA-256 is the signed one. A hacked hub can withhold updates, never sign one (D-41). Rotation: a release signed with a trusted key carries probes that trust the next key (a new `.pub` beside the old one).
- **The hub checks releases too**, with the same compiled-in keys, when one is added (the admin page's upload, `hub release add`, `hub install` with shipped probes): a wrong upload fails at once. It also refuses a release whose `protocol` it doesn't speak: that is where 0012's `min_protocol` check belongs, since the probe can't know its hub's range. Tests give `AppState::release_keys` their own key.
- **One store for downloads and updates.** D-53's unsigned `probes/` folder is gone: releases live in `<data folder>/releases/<version>/`, verified when added, and Add probe offers the approved release, else the newest. A hub built without a release key takes no release, so it offers no download either; `blentinelmake hub publish --with-probes` needs the key. Release files aren't in `hub backup`: they are build artifacts, added again from the build machine.
- **Approval is a hub record, in the UI** (`probe_update_approval`, migration `0008`), not `[updates] probe_version` in the TOML (PP-2: managed from the UI). None by default. The approved release can't be removed. Approve, add and remove are `ManageData` (Admins) and audited.
- **The local brake is a file, set by a command** (`probe updates off` writes `updates-off` in the probe's folder), since the probe has no config file any more (D-49). A probe with the brake on refuses the offer and says so, so the admin sees why on the hub; `probe updates on` forgets that refusal so the version is offered again.
- **Not in phase 1:** the update window (`[update] window`) and per-company approval, canaries and pause go to phase 2 (M4-01), set on the hub; `allow_downgrade` (signed rollback) too. The probe refuses any older version.
- **The switch.** Download to `probe.new` beside the binary (size capped by the manifest, 10 minutes), check the hash, run `probe.new --self-test` (prints its version, reads the identity, enrollment and buffer, reaches the hub; exit `2` when only the hub is unreachable, which is retried, unlike any other failure). Then `update.json` is written, and the binary switched: on Unix a hard link keeps the running file as `probe.prev` and one rename puts the new one in place, so there is never a moment without an executable; on Windows (no replacing a running file) the running one is renamed aside first. Restart: exit `75` under systemd (`Restart=always`); a Windows service stops with service-specific error `75`, and `probe install` now sets the recovery action to apply to such stops too (`set_failure_actions_on_non_crash_failures`, in `common::service`, so the hub's service gets it too); run by hand, the probe `exec`s (Unix) or starts (Windows) the new binary.
- **The watchdog** runs first in the monitoring loop: a start of the new version counts in `update.json`; at the 4th start without a delivered report, or 15 minutes (or 3 intervals) after the switch without one, `probe.prev` is put back, the version is remembered in `update-failed.json` and reported (`update_failed {version, error}` in the next report) until another version is installed. A download or self-test that may work later is retried 10 minutes later, up to 3 times, before it counts as a refusal.
- **Protocol v5, accepting v4.** Reports append `probe_version` and `update_failed`, omitted when absent, so a v4 report signs the same bytes and the hub accepts the current and previous version again (ENGINEERING §2.7). The hub records the version from every report (enrollment recorded it once before).
- **Platforms by OS and architecture:** a probe finds its binary through `common::release::Platform::current()` (`std::env::consts`), so a probe built for `x86_64-unknown-linux-gnu` updates to the release's static `musl` binary.
- **Verified** in the `blentinel-systemd-test` container with the hub and probes built by `blentinelmake` with a new release key: a probe service at 0.1.0 updated itself to a signed 0.1.1 after it was approved (confirmed by its next report, `probe.prev` kept); then 0.1.2, built to crash at start, was approved: the probe crashed 3 times under systemd, went back to 0.1.1, and the hub showed "0.1.2 not installed" and stopped offering it. The probe kept its identity throughout. Windows (the service's restart after a switch) is left to the owner.

## D-56 · The command channel: how M2-06 was built
**Status:** Accepted (implemented 2026-10-04); commands also reach the probe through its command poll since D-76 (M4-03), usually within seconds · **Implements:** proposal 0003 §4.3 · **Refines:** 0003's `probe_commands` sketch (an `expired` state, `args`), its initial list of command kinds; D-55's "accepting v4"

- **Commands ride in the signed answer to a report** (`ReportAck.commands`), like checks (D-49) and update offers (D-55): signed by the pinned hub key, sealed for the probe, bound to its report. The hub never connects to the probe (FP-1), so a command waits up to one report interval. Results come back in a later report (`command_results`, signed with it). The long-poll for near-real-time "run now" stays a separate item (M4-03).
- **At least once, not exactly once.** The hub sends a pending command in **every** answer until its result arrives or it expires, rather than once: the probe keeps only the answer to the newest report it delivered (a backlog flush answers many), and an answer can be lost. The probe remembers the last 256 command IDs it ran and skips repeats. That memory isn't on disk, so a probe that restarts between running a command and reporting its result may run it again: every command kind must be harmless to repeat. Because reports are delivered in order, the report carrying a result is always stored before the hub answers a later one, so in practice repeats need a crash in that window.
- **One kind for now: `run_checks`.** The probe runs its checks at once and sends the next report straight away (the path a new config already takes), with the result `ran N checks`. 0003's other kinds wait for the items that need them: `upgrade` (M4-01, phase 2 of self-update; phase 1 rides in `update`), `rotate_identity` (M2-13), `send_diagnostics` and `uninstall` (later, `uninstall` needing its own permission). `reload_config` is dropped: the config revision (D-49) already makes the probe pick up every change. A kind the probe doesn't know deserializes as `Unknown` (`#[serde(other)]`) and is reported as failed, so a newer hub can't make an older probe refuse the whole answer.
- **Protocol v6, accepting v5; v4 is refused** (ENGINEERING §2.7: current and previous; supersedes D-55's "accepting v4" for this hub). `command_results` is appended and omitted when empty, so a v5 report signs the same bytes. A v5 probe updates itself to v6 (D-55), which is why self-update came first; v4 probes, which can't, are installed again once (already the M2-15 advice). The hub sends commands only to a report of v6 or later (`COMMANDS_SINCE_PROTOCOL`); a queued command for an older probe fails with `can't take commands: update it` when it reports, so the user sees why instead of waiting for an expiry.
- **States:** `queued` → `delivered` (first sent; `delivered_at` keeps that time) → `succeeded` / `failed`, or `cancelled` (only while queued: once sent, the probe may already have run it). **Expiry is worked out on reading**, not stored by a sweep: a pending command past `expires_at` shows as `expired` and is no longer sent. A command waits 3 report intervals, at least 15 minutes, so "run now" doesn't happen hours later. A late result for an expired command is still recorded, since it says what the probe did.
- **Clicking twice runs once:** queuing a command that is already waiting (queued or delivered) returns that one, and only the first is audited (`command.queue`; `command.cancel`).
- **Bounds:** at most 10 commands per answer and 10 results per report (`MAX_COMMANDS_PER_MESSAGE`), result messages at most 500 characters; results for another probe's commands, or for ones already done, are ignored (`complete_command` matches the probe and a pending state).
- **Who:** queuing and cancelling are `ManageCompanies` within the user's company scope, like changing the probe's checks; seeing them is `ViewData`. A revoked probe takes no commands. Commands are listed on the probe page (the latest 20) and deleted with the probe; they aren't pruned otherwise (people queue them by hand, so they stay few).
- **Not done here:** long-poll (M4-03), commands for several probes at once, more kinds (above).

## D-57 · The one-line hub install: how M2-10 was built
**Status:** Accepted (implemented 2026-10-04); its release files gain `release.json`, and `hub upgrade` reads them (D-75) · **Implements:** proposal 0004 §4.3 · **Refines:** 0004's `get.blentinel.dev` host, `SHA256SUMS` "and its signature", the binary in `/usr/local/bin`; ROADMAP M2-10's `probe.sh` and `probe.ps1`

- **Only `hub.sh`.** The probe scripts M2-10 listed were already replaced by Add probe's hash-checked one-liners and `probe install` (D-53): a served script piped to a shell would duplicate them, harder to review. The owner chose to drop them.
- **GitHub Releases is the host** (owner's choice): `hub.sh` downloads from `github.com/mrjimmybob/blentinel/releases/latest/download/`, or `/download/v<version>/` with `BLENTINEL_VERSION`, or any folder with `BLENTINEL_RELEASE_URL` (a mirror; `file://` for an offline copy). Release CI is on hold (D-24), so `blentinelmake release hub` builds the files and `gh release create` uploads them by hand. Anonymous downloads need the repository's releases to be public.
- **Trust the release key, not the host.** `SHA256SUMS` lists every file of the release and is signed with the release key (D-55), which never leaves the build machine; `blentinelmake` writes the release public keys into `hub.sh` as PEM. The script checks the signature with `openssl pkeyutl -verify -rawin` (OpenSSL 1.1.1 or later: Debian 11, Ubuntu 20.04 and newer), before downloading anything large, then each file with `sha256sum -c`, and only then runs `hub install`. It fails closed: no `openssl`, no install, rather than a switch to skip the check. The signature is raw (64 bytes) because that is what `openssl` reads. The script itself still comes over HTTPS from the host; the docs say how to check it first. The hub then checks the shipped probe release against its compiled-in keys, as for any release.
- **A static musl hub**, so one file runs on any Linux with systemd, like the probe. That needed `lettre` (e-mail) on **rustls** instead of native-tls/OpenSSL (`ring`, the platform verifier: the system's trusted certificates, as before); the hub has no OpenSSL left. The hub is built for musl by cargo-leptos with `LEPTOS_BIN_TARGET_TRIPLE` on Linux with `musl-tools`; `blentinelmake` refuses on other systems rather than half-building. Targets: `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (`uname -m` picks); the aarch64 hub needs an aarch64 machine or a cross C toolchain (SQLite and *ring* are C).
- **`hub install` does the installing**, as in D-52: the script only downloads, checks, and puts `probes/` beside the hub binary in a temporary folder; `hub install` copies itself to `/opt/blentinel/hub`, writes the config and service, and adds the probes. So running the script again upgrades in place, and options after `sh -s --` are `hub install`'s.
- **Files of a release:** `hub-<target>`, `probes.tar.gz` (the signed probe release, flat, as `hub release add` takes), `hub.sh`, `SHA256SUMS`, `SHA256SUMS.sig`. Names carry no version, so `latest/download/` works.
- **Verified** with a release built by `blentinelmake release hub` in the `rust:1-bookworm` image with a throwaway release key (a static-pie musl hub, 15.6 MB), installed by piping its `hub.sh` to `sh` as root in a fresh `blentinel-systemd-test` container (`BLENTINEL_RELEASE_URL=file://…`): the service ran, HTTPS answered, the probe release was added and served; options after `sh -s --` reached `hub install`; running it again upgraded in place. Refused, installing nothing: a changed binary or `probes.tar.gz` (checksum), a re-written `SHA256SUMS` (signature), a script trusting another key, a missing file, no `openssl`, not root. The `wget` fallback wasn't exercised (the image has curl). Nothing was uploaded to GitHub.

## D-58 · Docker image on hold until a release candidate
**Status:** Accepted (2026-10-05) · **Affects:** ROADMAP M2-11

- The owner put M2-11 (Docker image and compose file) on hold: it packages a release, and the core system is still being built (the web UI isn't polished yet, nor planned). An image made now would have to follow every change to the core for no user.
- Nothing depends on it: `hub install --no-service` already writes the files a container needs, and the one-line install (D-57) covers a fresh server. It comes back when a release candidate is prepared.

## D-59 · Monitoring only: pending updates are reported, never installed
**Status:** Accepted (2026-10-05) · **Affects:** ROADMAP M5-11, proposal 0006 #9

- **Asked:** could the probe run Windows Update or the Linux package manager on its machine? Technically yes without breaking FP-2 (the Windows Update Agent COM API in-process; PackageKit over D-Bus, with a polkit rule granting the probe only that), but it would turn Blentinel from monitoring into remote management (RMM): a hacked hub could trigger updates and reboots across every client, long jobs and reboots would need new command states, and a bad patch becomes the product's fault.
- **Decided by the owner:** Blentinel stays **monitoring only** until it ships as a finished monitoring product. The probe **reports** pending updates and never installs them. Patch management may be reconsidered after that, as its own proposal.
- **How:** a check kind (M5-11), added per probe like any other, not a metric every probe always reports: it gets thresholds, silences and history like other checks, a Windows Update search is slow, and not every monitored machine is the MSP's to patch. Check templates (M5-12) can add it to new probes by default. Read-only: no tools run (FP-2), no rights beyond what reading needs. It reports what the OS already knows (its own update timers refresh the lists), with when that was.

## D-60 · Backups the hub makes by itself: how M2-12 was built
**Status:** Accepted (implemented 2026-10-06) · **Implements:** proposal 0004 §4.5 · **Refines:** ENGINEERING §2.6 "pre-migration backup (SQLite: file snapshot)"

- **On by default:** daily at 03:00 UTC, the last 7 kept. A new hub is backed up without anyone setting it up; the admin page (Backups) turns it off or changes the hour (UTC, like every time the hub shows) and how many to keep (1 to 365). Once a day only: a weekly or hourly choice can come when someone needs it.
- **What's where:** the schedule is a database record (PP-2: managed from the UI); only the folder is in the config, `[backups] path` (default `backups` in the data folder), because it's a place on the server like `state_dir`. Choosing a path from the browser would let a web session write the hub's private key anywhere on the server.
- **One format for every backup:** the daily backup, **Back up now** and the backup before an upgrade are all `hub backup` files (D-52: database, key, TLS, config), so `hub restore` takes any of them. The kind is in the file name (`blentinel-hub-<date>[-scheduled|-pre-upgrade].tar.gz`), so the folder is the only record of what exists: files copied in or deleted by hand are seen as they are, and no table can disagree with the disk. Retention is per kind: scheduled ones beyond "keep", pre-upgrade ones beyond the newest 3; manual ones are never deleted automatically.
- **When:** the scheduler looks every minute; a backup is due when none ran since the latest scheduled hour, so a hub that was off at that hour catches up when it starts. A run is recorded **even if it fails**, so a failing backup isn't retried every minute; the admin page shows the error, the log has it, and **Back up now** retries by hand. One backup at a time. The database is streamed into the archive and compressed off the async threads (M2-08 read it into memory).
- **Before an upgrade (proposal 0004 §4.5):** at startup, on a hot reload that switches databases, and for `hub user`, a database with an older schema is backed up (a full backup, read through a read-only connection, so nothing changes first) and only then migrated. If the backup fails, nothing is migrated and the hub doesn't start. If a migration fails, the hub moves the half-migrated database (and its WAL files) to `<data>/failed-upgrade-<date>/`, writes the backed-up one back, and exits saying what happened and where both are. A new database, or one already current, gets no backup.
- **Migrations run over a connection of their own,** closed whatever happens (`Migrator::run_direct`). Through the pool, the connection of a failed migration went back to the pool later, in the background, so the file was still open and Windows refused to move it aside; the test that fails a migration found this.
- **Downloading asks for the password again** (owner's decision): each backup holds the hub's private key, so a stolen session alone mustn't be enough. **Download** asks for the user's own password (`ManageData`, like the other backup actions); wrong ones count against the same lock as failed logins (D-43) and are audited (`backup.download_failed`). A right one gives a **one-time link**, valid for 60 seconds and only for that user's session (`backup.download`); the browser saves the file from it, streamed from disk, so a large backup isn't held in memory on either side. The links live in memory (a restart forgets them: ask again), stored as SHA-256 hashes like session tokens. Backups can also leave the server through `[backups] path` on another disk or share.
- **Not done here:** an alert when the daily backup fails (it shows on the admin page and in the log), `hub status`/`doctor` showing the last backup, backups for an external database (M7, logical export), off-server destinations (S3, SFTP).

## D-61 · Rotating the hub's certificate and key: how M2-13 was built
**Status:** Accepted (implemented 2026-10-06) · **Implements:** proposal 0002 §4.5 · **Refines:** D-47 (the TLS pin), D-01 · **Supersedes:** D-56's "commands go only to v6 probes" (no accepted probe is older now)

Decided by the owner before building it:

- **The TLS pin is the SHA-256 of the certificate's public key (SPKI)**, not of the whole certificate (reverses part of D-47, back to D-01). A certificate renewed with the same key, as Let's Encrypt can (M2-14), keeps the pin, so a renewal needs no rotation; rotation runs when the key really changes. The probe reads the key with rustls's own parser (`rustls-webpki`, already in the build: FP-2 holds). The hub still logs the whole-certificate fingerprint for comparing with a browser. Enrollment codes are version 3 (the pin's meaning changed); version 2 codes are refused with "make a new one".
- **The hub switches when every active probe holds the next pins**, checked once a minute, **or when an admin clicks "Switch now"**. The confirmation says how many probes aren't ready; they then need enrolling again.
- **The window (14 days by default, 1 to 90, set when starting) is a deadline that asks, not one that acts:** past it, a rotation still waiting is flagged once in the log and stays flagged on the page with the probes holding it up and why; nothing is cut off without a click.
- **One mechanism for the certificate, the key, or both.** The key means the X25519 key and the Ed25519 signing key derived from it (D-48): they rotate together.

How it works:

- **Files, not config:** the next key and certificate are written beside the current ones (`hub_identity.next.key`, `hub_tls_cert.next.pem`, `hub_tls_key.next.pem`), and the switch moves them in place (an atomic rename each), so `[server]` never changes (PP-2). The certificate is a new self-signed one with a new key pair; uploading one comes with ACME (M2-14). The database records the rotation (`hub_rotations`) and what each probe said it holds (`probes.pins_current`, `pins_next`).
- **Trust chain:** the next pins ride in the signed answer to a report, signed with the **current** key, which the probe already pins. Nobody else can plant pins, even on plain HTTP. After the switch the answer, signed with the new key, says `switched`; a probe that holds the next pins trusts it and drops the old ones.
- **Pins as digests:** reports (protocol v7) carry SHA-256 digests of the pins the probe holds, current and next (`common::pins::HubPins::digest`); the hub compares them with its own, so readiness needs no extra round trip. A probe without a certificate pin (plain HTTP, or a proxy) matches without it and keeps not pinning one.
- **Buffered reports:** after a key switch the hub keeps `hub_identity.prev.key` and opens reports with it too, for as long as a probe may still deliver one sealed before it switched (the buffer's age limit plus clock skew); then it deletes it. The probe keeps its buffer when it switches keys (its black box records the new key instead of discarding the queue).
- **In place:** the HTTPS listener loads the new certificate without a restart (`TlsHandle`, axum-server's reload), so connections in flight continue.
- **Codes:** enrollment codes made before the switch carry the old pins, so the switch revokes them (counted in the audit entry). Codes made during the announcement carry the current pins and the probe is then announced the next ones like any other.
- **Crash safety:** at startup, a rotation still `announcing` whose next files are partly gone was switching when the hub stopped: the remaining moves are finished and it is recorded as switched. Backups include the next and previous files; a restore moves any existing ones aside, so the files always match the restored database.
- **Protocol v7, accepting v6:** a v6 probe reports no pins, so it is "not ready" ("runs an older probe") until it updates itself. **But a v6 probe enrolled over HTTPS can't update to v7:** its `enrollment.json` holds a whole-certificate hash, which v7 doesn't read, so the new binary fails its self-test and rolls back. Pre-alpha (D-30): such probes are installed again once with Add probe's command; probes on plain HTTP update themselves.
- **Commands:** with v6 the oldest accepted version, the "too old for commands" path (D-56) could no longer run, so it and its constant are removed.
- **Not for a stolen key:** whoever has the key can sign a rotation too. After a theft the hub gets a new key by hand and every probe is enrolled again (documented in security.md).
- **Verified** by unit tests (pins, readiness, what each answer says, the probe's enrollment changes), integration tests (announce, all-ready switch, switch now, overdue flagged once, cancel, certificate files swapped and new codes pinned, previous key kept then deleted, an interrupted switch finished at startup), and a browser test with the real probe: certificate and key rotated, the hub switched in place (the served certificate changed without a restart), and the probe kept reporting.
- **Not done here:** uploading a certificate (ACME is M2-14), rotating only some companies' probes, an e-mail when a rotation is overdue, showing a rotation in `hub status`.

## D-62 · A certificate from Let's Encrypt: how M2-14 was built
**Status:** Accepted (implemented 2026-10-06) · **Implements:** proposal 0004 §4.4 option 2, D-03's "optional ACME/Let's Encrypt" · **Challenges:** proposal 0004 §4.4's `[tls] acme_domain` (settings on the admin page instead) · **Builds on:** D-61

- **Issued for the hub's own key.** The certificate request is signed with the existing `hub_tls_key.pem`, so the issued certificate has the public key probes pin (D-61). Turning ACME on, every renewal, and turning it off again (the hub then writes a self-signed certificate **for the same key**) change nothing for probes: no rotation, no enrolling again. A new key comes only from a rotation. The hub refuses what an authority sends back unless it is for its key and its names. Probes keep trusting the pin, never the authority (FP-3).
- **Settings on the admin page, not in the config** (PP-2: managing, not bootstrapping), as a database record (`acme_settings`); the account key and the certificate are files (`hub_acme_account.json`, beside the others, in backups). Names (1 to 10 DNS names; no IP addresses, no wildcards: they need DNS validation), an optional contact, the authority (Let's Encrypt, its staging service, or any ACME directory URL with its root certificate if private: a company's own CA such as step-ca), the validation method, and the terms accepted.
- **Validation: TLS-ALPN-01 (default) or HTTP-01, the admin's choice.** TLS-ALPN-01 runs on the HTTPS listener (the authority connects to 443): the listener now chooses its certificate per handshake (`tls::TlsHandle`, a rustls certificate resolver) and answers `acme-tls/1` with the challenge certificate only while an order waits. HTTP-01 runs on the plain-HTTP listener (port 80 from outside); `/.well-known/acme-challenge/` is the one path it serves besides the probe endpoints. Neither can be tested from the hub itself (only the authority sees the hub from outside), so the page says which hub port the authority's port must reach, and a failure shows the authority's own reason. DNS-01 (wildcards, hubs not reachable from the internet) would need a DNS provider's API: not done.
- **The resolver replaces axum-server's reload** for a rotation's switch too: one way to change what the listener serves, in place.
- **Renewal:** the hub looks every hour (and 20 s after starting, and when a rotation announces a new certificate) and renews when a third of the certificate's life is left, as Let's Encrypt advises; this also fits the shorter lifetimes coming. Saving new names or another authority asks at once, and so does saving while the last attempt had failed (the admin fixed something); a new contact or validation method otherwise waits for the next renewal (authorities limit issuance: Let's Encrypt, 5 a week for the same names). **Renew now** asks at once, whatever the age. A failure is recorded and shown; the hourly look then waits 1 h, doubling up to 24 h (failed validations count against the authority's limits). One order at a time.
- **Rotation:** while a rotation of the certificate announces (D-61), the next key gets its own certificate from the authority, so the switch serves a trusted one; it is issued at once, not at the switch.
- **instant-acme** (pure Rust, rustls with aws-lc-rs like the rest of the hub's TLS, so the static musl hub still builds; the authority's own HTTPS is checked with the OS's certificate store through rustls-platform-verifier, as for e-mail). The authority is behind a trait (`service::acme::CertificateAuthority`), so the integration tests use their own CA.
- **`hub doctor`** warns with 30 days left, or a quarter of a short certificate's life, so only a late renewal warns.
- **Verified** by unit tests (settings, renewal timing, back-off, what a change of settings needs, request and certificate details, challenge answers), storage conformance, integration tests (on: issued for the same key and served in place; renewal at a third; a failure shown, retried later and later, Renew now; off: self-signed with the same key; settings checked against the listeners; a rotation's next certificate issued before the switch; HTTP-01 answers; backup and restore keep the account), a real-socket test of the TLS-ALPN-01 answer (critical `acmeIdentifier` with the digest, only for `acme-tls/1`, the hub's certificate for everyone else), a browser test of the section (a refused name, an unreachable authority shown, off again), and by hand against **Pebble**, Let's Encrypt's test server: the real hub got certificates over TLS-ALPN-01 and HTTP-01, renewed, and went back to self-signed with the pin unchanged throughout; an unreachable name showed the authority's reason.
- **Not done here:** DNS-01 and wildcards, External Account Binding (ZeroSSL, Google), ACME Renewal Information (ARI), certificate profiles, revoking the certificate when turned off, an e-mail when renewal fails (the page and the log say so).

## D-63 · Flap suppression: how M3-02 was built
**Status:** Accepted (implemented 2026-10-06) · **Implements:** proposal 0005 §2.2 · **Challenges:** 0005 §2.2's "defaults set globally" (per check only, for now) · **Changes:** alerting on every change of status (the behaviour `alert_state.rs` pinned until now)

- **A pure state machine** (`service::alerts::flap::next(state, failing, policy) -> (state, transition)`): the confirmed state and a streak of results against it. A result that agrees resets the streak; `fail_after` failed results in a row confirm a failure, `recover_after` good ones a recovery, and only then is anyone told. A check never seen counts as fine, so one that is down from the start alerts after `fail_after` results like any other. Tested exhaustively (every sequence of up to 8 results for every policy up to 4) against its definition, rather than with property-based tests (no new dependency, and the space is small).
- **Default 2 and 2**, as the proposal says: a blip is ignored, a real outage alerts one interval later.
- **Per check, on the check** (`checks.fail_after`, `recover_after`, `NULL` = the default), set in the check's dialog; copying checks copies them. Hub-only: they aren't in the probe's config, so its revision doesn't change. **No global or per-company default yet:** alert settings still live in `[alerts]` in the config file until M3-03 moves them to the UI (PP-2), and adding new file settings now would mean moving them again; a default changeable there comes with that page. 1 gives the old behaviour.
- **Thresholds too:** a usage threshold (disk/CPU/memory) is the same machine over "at or over the limit", with the check's numbers: a CPU spike is the classic flap. Its re-arm (back under for `recover_after` reports) sends nothing, as before. A threshold e-mail that fails to send is retried with the next report, as before.
- **Stored** in `alert_states` as before (`last_status` = the confirmed status, so existing rows keep their meaning) plus `streak`. A check that was never anything but fine leaves no row.
- **Silences** keep tracking state; a silenced failure tells nobody; a confirmed recovery ends the silence (not one good result).
- **One report of a probe at a time (a bug the counter exposed):** ingest evaluated each report's alerts in a task of its own, so reports arriving together (a probe delivering its buffer after an outage) were evaluated concurrently, read the same streak and lost a result: an outage could go unreported. `alerts::spawn_in_order` still answers the probe at once, but each probe's evaluations wait for the previous one, in the order the reports arrived (fixed when they arrive, not when a task starts). A regression test failed without it.
- **What doesn't change:** the dashboard and the probe page still show each result as it is (M3-01 brings the 4-state status); probe expiry has its own timeout and isn't counted.
- **Not done here:** the optional "Flapping" state (K changes in a window), a global or per-company default, showing "failing, 1 of 2" on the page.

## D-64 · Four statuses and the threshold hierarchy: how M3-01 was built
**Status:** Accepted (implemented 2026-10-06) · **Implements:** D-16, proposal 0005 §2.1 · **Challenges:** proposal 0005 §2.1's "warn/crit pairs" (each limit inherits on its own) · **Removes:** `[alerts.thresholds]` and the company overrides' thresholds from the config file (D-30: pre-alpha, no import)

- **One status, judged once, on the hub, when a result arrives** (`service::thresholds::evaluate_report` at ingest): Down if the probe says it failed or a down limit is reached, Warning if a warning limit is, else Up. It is **stored with the result** (`check_results.status`, and `reason` when a threshold decided), so the dashboard, the probe page, uptime and alerts read the same thing, and history keeps the status it had: changing a threshold applies from the next report, it doesn't rewrite the past. **Unknown** is shown, not stored: the checks of an expired probe (the devices query answers Unknown while its heartbeat is expired). Dependencies (Unknown behind a dead parent) are M3-07.
- **What thresholds apply to:** the usage of `cpu`, `memory` and `disk` checks (%), the latency of `ping`, `http`, `tcp` checks (ms). TLS expiry joins with M5-01. The probe still only says whether a check worked (FP-2: all logic on the hub).
- **The hierarchy: check → site → company → the hub's default → built in**, **limit by limit**: each warning and down limit comes from the lowest level that sets it, so a company can set only a disk warning and keep the hub's down limit. A level can set a limit **off** (`"off"` in JSON and in the forms) to switch off what a higher one sets. Built in: the old alert thresholds as warnings (CPU 95 %, memory 90 %, disk 90 %), disk down at 97 %, no latency limits (they depend on the link). Validation is one function for the forms and the API: 0–100 for %, 0–600 000 ms, warning below down when both are numbers at the same level.
- **Set in the UI** (PP-2): the admin page (**Thresholds**: the hub's default, `ManageData`), the company page (**Thresholds**, the company or one of its sites, `ManageCompanies`, scoped), the check dialog (its own limits). Each form shows what an empty limit inherits as its placeholder, and where it comes from on hover. The config file's thresholds are gone; an old section is ignored. Audited as `thresholds.update`; a check's are in `check.update`.
- **Alerts:** a check has **one** alert state now (its confirmed status), not a separate threshold state: M3-02's counting extended to three statuses (to Up takes `recover_after` results in a row, to Warning or Down `fail_after`; the news is the latest). E-mails: DOWN, WARNING (new), RECOVERED, with the reason (the threshold, or the probe's message). Back under a warning limit now sends RECOVERED (it used to re-arm silently). The old `-threshold` alert rows are deleted by the migration.
- **Severity:** a new **warning** level, below degraded: a stopped probe (not knowing) outranks a warning. Amber now means warning; degraded is grey. Uptime counts Warning as up (it works).
- **Not done here:** TLS-expiry thresholds (M5-01), dependencies (M3-07), per-check thresholds on metrics other than the headline one, re-evaluating stored history after a change, a Warning colour in the uptime chart.

## D-65 · Maintenance windows and wider silences: how M3-04 was built
**Status:** Accepted (implemented 2026-10-07) · **Implements:** proposal 0005 §2.4, D-44's "site timezone with maintenance windows" · **Decided by the owner before building it** (the recommended options)

- **Two tools.** A **window** is planned: once, weekly (chosen days) or monthly (a day of the month, clamped to its last day; or the 1st–4th or last weekday), at a local start time for a length, at most 7 days. A **silence** is ad hoc: from now, for a time or until removed (a check's also until it recovers). Both cover **a check, a probe, a site or the whole company**, always within one company.
- **During a window, statuses are kept and shown; only notifications stop.** Results are judged as usual and stored with a `maintenance` flag (at ingest, like the status, D-64: a window added later doesn't rewrite history). The device list marks them.
- **Still wrong when it closes: alert then** (owner's choice). The alert state keeps what people were told; news found during the window is held, its streak left one short (`flap::held`), so the first report after the window that still says so alerts at once, and what recovered meanwhile alerts nobody. A probe that stops reporting during a window (or while silenced, at any scope that covers it) alerts once it closes: every sweep now looks at every expired probe that hasn't alerted, not only the newly expired.
- **Uptime leaves windows out** (owner's choice): `check_results.maintenance` rows are counted apart (`maintenance_count`), and the chart shades those buckets. Silences don't change uptime.
- **Time zones:** each window stores its own IANA zone, prefilled with its site's (owner's choice; companies have none). Sites gain `timezone` (default `UTC`), validated on the hub. The zone rules are `chrono-tz` (compiled in; no system database needed, PP-3). Local times follow daylight saving; a start in the skipped hour opens when it ends, one in the repeated hour the first time round. The forms validate the rest with the shared model (no chrono in the browser).
- **Silences re-keyed:** `alert_silences` gets `company_id` and `(company_id, scope_type, scope_id)` as its key (a site ID is only unique within its company); the table is made again, keeping check silences. One query answers "is this check, or this probe itself, silenced by any scope". `POST /api/silence` takes a company and a scope (pre-alpha, D-30: no old form).
- **Who:** Technicians (and up) add and remove windows and silences of their companies (`Silence`); Viewers and Clients see the page (`ViewData`, scoped). Audited.
- **One page per company** (owner's choice), *Maintenance and silences*, linked from the company page: windows (covers, schedule, why, next opening or "Open now") and silences, each with a form.
- **Verified** by unit tests (weekly in local time, daylight saving both ways, the 2nd Tuesday and the last Friday, the 31st in a short month, one-off, bad zones, scopes, held news), storage conformance (silences of every scope, another company's site of the same name), integration tests (API and validation, alerts held then sent after the window, recovered within it: nothing, uptime counts, probe expiry after the window, site time zones, scoping) and a browser test of the page.
- **Not done here:** windows shown on the dashboard or probe page, editing a window (remove and add), maintenance outside one company, exceptions to a recurring window, an e-mail when a window opens or closes.

## D-66 · Alert channels and routing in the UI: how M3-03 was built
**Status:** Accepted (implemented 2026-10-07) · **Implements:** proposal 0005 §2.6, D-04 (alert settings) · **Decided by the owner before building it** (the recommended options, Telegram added; WhatsApp left out)

- **Channels:** e-mail, a signed webhook, Microsoft Teams (an Adaptive Card through a Workflows webhook), Slack, Discord and Telegram (a bot token and a chat ID). WhatsApp was left out: its Business API needs a Meta business account, approved message templates and per-message fees, which is too much for one MSP to set up; Telegram is free and takes a minute. ntfy and Pushover stay in M3-08.
- **No import, no TOML** (owner's choice, D-30): `[alerts]` is removed from the config (an old section is ignored), and a new hub has no channels. The mail server and the hub's address are settings on the same page.
- **Routing rules, in order:** each matches alert kinds, companies, sites (`company/site`) and check kinds (empty: all; check kinds don't match a probe's alert) and names channels; an alert goes to the channels of every matching rule until one with **stop** matches, each channel once. Kept as JSON rows (`alert_routes`) and saved as a whole: the editor is a list, and a partial save of a reordered list has no meaning. The matching is `model::route`, a pure function shared by the hub and tests.
- **The engine raises, the dispatcher delivers.** The alert engine builds one `Notification` (kind, company, site, probe, check, why) and gives it to the `Notifier`; production's is the `Dispatcher`, which routes it and delivers to each channel in the background. What each kind of channel sends is data (`Outgoing`: an e-mail, or a POST with its exact body), made by a pure function and delivered by a `Transport` (reqwest with rustls; lettre), so tests check every shape without a network and one test delivers a signed webhook over a real socket.
- **Retries:** up to 3 tries, 1 then 4 minutes apart, in memory: a restart drops them (a durable queue is M3-06's escalation work). Each channel records its last delivery and error, shown on the page. Errors never contain a URL (`reqwest::Error::without_url`): for chat channels, the URL is the secret.
- **Secrets are sealed at rest** with ChaCha20-Poly1305 and a key of their own, `hub_data.key`, beside the other key files and in backups (restored with them). Not the identity key: it changes when it rotates (D-61) and the secrets must not. Secrets are write-only: answers say only whether one is set; a change without one keeps it, except when the kind changes. The audit log records that a secret changed, never the secret.
- **Webhook signatures:** HMAC-SHA256 over `<timestamp>.<body>` (the timestamp in its own header), as Stripe and Slack sign theirs, so a receiver can refuse a replay by age without keeping state.
- **E-mail security is explicit** (STARTTLS, TLS, none), which fixes the old code using implicit TLS on port 587.
- **Who:** Owners and Admins (`ViewAdmin` to see, `ManageData` to change): channels hold secrets and decide who is woken at night. Technicians keep thresholds, windows and silences.
- **Verified** by unit tests (each channel's shape, signatures, routing, validation, sealing), storage conformance (settings, channels, rules), integration tests (the API, secrets never answered or logged, Send test, pruning rules, routing with stop and disabled channels, 3 tries and the recorded error, a signed webhook over a socket, an error without its URL) and a browser test of the page.
- **Not done here:** per-company channels for Technicians, quiet hours, rate limits, a durable retry queue, delivery history beyond the last one, WhatsApp, ntfy and Pushover (M3-08).

## D-67 · Incidents, acknowledgement and the timeline: how M3-05 was built
**Status:** Accepted (implemented 2026-10-07) · **Implements:** proposal 0005 §2.3 (re-notification and escalation are M3-06) · **Built with the recommended options** (the owner said continue)

- **An incident is one outage**, of a check or of a probe that stopped reporting (`check_id` `NULL`), from the news that alerts to the news that recovers. Warning and Down are one incident: `status` is now, `severity` the worst it got (the proposal's single `severity` column couldn't say both). A new outage is a new incident.
- **It follows the news, not the notification.** The alert engine's news (after flap suppression, D-63, and maintenance holding it back, D-65) opens, changes and closes it, also when a **silence** keeps everyone from being told: a silence means "don't tell me", not "it didn't happen", and the timeline says nobody was told. A probe's incident opens when its PROBE EXPIRED alert is sent (a silence or window delays that alert, as before) and closes when it reports again (at ingest, like re-arming the alert, D-27). Deleting a check closes its incident; deleting a probe's or company's data deletes theirs.
- **The names are kept as they were when it opened** (company, site, probe, check): the history reads as it happened, and the list needs no joins.
- **One open incident per check, and per probe,** enforced by a partial unique index (`WHERE closed_at IS NULL`), not only by the code.
- **Acknowledge** once per incident, while open, with an optional note; **notes** any time. Who: `Silence` (Technicians and up), as for silences: the people who handle the outage. Viewers see; **Clients** see their companies' incidents and the hub's events, not what people wrote (notes, acknowledgement notes), like a company's internal notes.
- **Alerts point to their incident:** the link goes to `/incidents/{id}`, a recovery says how long it lasted (proposal §2.7), and the webhook body carries `incident`.
- **Acknowledging changes no alert yet.** What it stops (re-notification) and escalation come with M3-06, which builds on this.
- **Verified** by unit tests (durations, notes, the links and "Lasted"), storage conformance (open, change, close, acknowledge once, one open per check, scope), integration tests (one incident per outage, Warning and Down, silenced, a probe stopping and coming back, deleting a check, acknowledge and notes, conflicts, scope, Clients, removing a probe) and a browser test (acknowledging and noting the e2e probe's always-down check; a Client's view).
- **Not done here:** assigning to someone else, re-opening, merging incidents, incidents on the dashboard and probe page, an alert when someone acknowledges, SLA reports.

## D-68 · Check dependencies: how M3-07 was built
**Status:** Accepted (implemented 2026-10-07) · **Implements:** proposal 0005 §2.5, D-16's "Unknown: an unreachable parent dependency" · **Built with the recommended options** (the owner said continue)

- **A check depends on one check of the same probe** (`checks.depends_on`, cleared when that one is deleted). The same probe, because the parent's result then comes in the same report: whether a child is reachable never depends on another probe's timing or clock. A site with two probes links checks within each. Chains are allowed (router ← switch ← NAS); a check behind itself, a loop, or another probe's check is refused, on the hub (`service::dependencies::validate`).
- **Unreachable is decided per report, on the parent's result** after thresholds: Down in that report makes the checks behind it Unknown in that report. Not on the parent's alert state: the first failures of a router (before its own alert) are the ones that would otherwise count against everything behind it, and a single lost gateway ping now costs the children nothing. The root cause is the farthest ancestor that is Down.
- **Unknown results are stored** (`check_results.status = 'Unknown'`, with `unreachable: Gateway is down` as the reason): the probe page shows why, and uptime leaves them out on its own (it counts Up/Warning and Down only). Unknown was "shown, never stored" (D-64): it is now also stored for this.
- **An Unknown result leaves the check's alert state as it was** (no news, no streak): it says nothing about the check. A check that was already down (and alerted) stays so, with its incident open; one that is still failing when the parent is back alerts after its usual results in a row.
- **The parent names what is behind it:** its DOWN alert and incident say `unreachable behind it: NAS, Printer`, so the one alert says how much is affected. The children open no incidents (proposal: "the incident names the root cause").
- **Hub-only** like the alert settings: the probe's config (and revision) doesn't change.
- **Copying checks keeps the links** between the copies (new IDs mapped; created first, linked after).
- **Fixed on the way:** the probe page's Disable/Enable sent only the name, spec and flag, and the API takes what is missing as its default, so it reset a check's alert-after counts and limits (since M3-02/M3-01). It now sends the whole check (`switched`, unit-tested).
- **Verified** by unit tests (unreachable behind a down parent, the root cause in a chain, a parent without a result, validation, the toggle), integration tests (only the gateway alerts and names the NAS, one incident, a check still down after the gateway is back alerts, a gateway blip, the root cause in a chain, Unknown stored and out of uptime, the API's refusals, deleting the parent, copying) and a browser test (setting and removing **Behind** in the dialog).
- **Not done here:** suggesting the gateway automatically (the probe doesn't report its default gateway yet), dependencies across probes, several parents, showing the tree.

## D-69 · Reminders and escalation: how M3-06 was built
**Status:** Accepted (implemented 2026-10-07) · **Implements:** proposal 0005 §2.3 (re-notify, escalate) · **Decided by the owner before building it** (the recommended options)

- **On each routing rule** (owner's choice): *remind every N minutes* and *escalate after M minutes to these channels*, both off by default. Urgency stays next to who is told: ACME's outages can page every 30 minutes while warnings go to a mailbox once. A hub-wide setting would treat every customer and every warning alike.
- **Escalation is a second set of channels on the same rule** (owner's choice), one level, once per incident. When an alert matches several rules: the shortest reminder interval, to every reminding rule's channels; the earliest escalation, to every escalating rule's channels.
- **What stops them:** acknowledging (what it is for, D-67) and recovery. Silences and open maintenance windows pause them; an incident that opened silenced (`incidents.announced = 0`) is never reminded: nobody was told the first time.
- **The recovery of an escalated incident also goes to the escalation's channels** (owner's choice), so whoever was woken hears it is over: the incident's reference carries the kind it was escalated as, and the dispatcher adds that kind's escalation channels to a recovery.
- **In the database, not in memory:** each incident keeps `reminders`, `last_reminded_at` and `escalated_at`, and a sweep (with the probe-expiry one, every 10 s) works out what is due from them and the current rules. A restart neither loses nor repeats one; each is recorded before it is sent (as probe expiry is, D-27), so a slow channel can't make it twice. Delivery itself keeps M3-03's in-memory retries (seconds to minutes).
- **What they say:** a reminder `… NAS STILL DOWN`, with `Open: for 2 h, not acknowledged (reminder 3)`; an escalation `… NAS DOWN (escalated: not acknowledged for 1 h)`; webhooks `event` `reminder` / `escalation` and `kind`. They name their channels (`Nudge`), so routing isn't redone for them. Each is on the incident's timeline (`reminded`, `escalated`).
- **Limits:** reminders every 15 minutes to once a week; escalation after 5 minutes to a day; no cap on reminders (acknowledging is the way to stop them).
- **Not done here** (owner's choice): acknowledging from the alert itself without logging in (signed one-time links change the security model: a separate item), on-call schedules, quiet hours, several escalation levels.
- **Verified** by unit tests (several rules' reminders and escalations, a rule that stops, the webhook event, the headline), storage conformance (the unacknowledged list, reminders, one escalation, not an unannounced one), integration tests (reminded on time and once per interval, escalated once to its channels, acknowledging and recovery stop them, the recovery reaches the escalation's channels, a reminder goes to its channels only, a silenced outage never, a silence pauses them, the settings' checks, removing an escalation's channel) and the browser test of the Alerts page (setting them, a refused interval).

## D-70 · ntfy channels, and no Pushover: how M3-08 was built
**Status:** Accepted (implemented 2026-10-08) · **Implements:** proposal 0005 §2.6 (M3-08) · **Decided by the owner before building it**

- **ntfy, on any server** (owner's choice): the public ntfy.sh or the installer's own. The channel's target is the full topic URL (`https://<server>[/path]/<topic>`), so no server is built in: the project's own is `ntfy.blentinel.net`, an installer's is theirs. The access token is optional (`ChannelKind::secret_required`, new: the first optional secret) and sealed like the others.
- **Never required** (owner's requirement, PP-3): like every channel, ntfy is something an installer adds; the hub runs and alerts without it.
- **JSON publishing** to the server, naming the topic: ntfy's header form takes ASCII only, and customer names aren't. The token goes as a bearer token. Priority follows the alert: outages, reminders and escalations urgent (5); warnings and stopped probes high (4); recoveries and tests default (3). `click` opens the incident when the hub's address is set.
- **Topics on a public server are guessable passwords:** the form's **New random topic** makes `blentinel-` and 16 hex characters from the browser's secure random source, on the server already typed (ntfy.sh only as the example when the field is empty), and the form warns (doesn't refuse) about a topic under 16 characters. The guide says what passing alerts through ntfy.sh means (a third party sees them for 12 hours) and [install/ntfy.md](install/ntfy.md) sets up a self-hosted server (Docker, a reverse proxy, a write-only account for the hub and a read-only one for phones).
- **No Pushover** (owner's choice): it is paid and closed source. It would be a small channel, but the project keeps to free, open ones; a webhook reaches it if someone needs it.
- **Verified** by unit tests (topic URLs, the optional token, the JSON, priorities, the random topic and the warning), an integration test (Send test to a local ntfy-like server over a socket, with and without a token) and the browser test of the Alerts page.

## D-71 · The daily digest: how M3-09 was built
**Status:** Accepted (implemented 2026-10-08) · **Implements:** proposal 0005 §2.6 (M3-09) · **Built with the recommended options** (the owner said do it)

- **One digest for the hub**, set on the Alerts page: an hour (0–23) local to an IANA time zone (`chrono-tz`, as maintenance windows, D-65), and the **e-mail channels** it goes to (the roadmap's "digest e-mail"; long text suits mail, not chat). No channels: off. Kept apart from the mail server's settings (its own table and form), which have their own write-only password.
- **Every day, also when all is well:** a digest that stops arriving says the hub (or its mail) is in trouble, which nothing else would. The subject sums it up (`all well`, or the counts).
- **What it says:** open incidents (how long, who acknowledged, or NOT ACKNOWLEDGED, or silenced); probes not reporting; the last 24 hours (opened, closed with durations, longest first, at most 20); uptime over 24 hours per company; a link to the incidents when the hub's address is set. Times in the digest's zone. Uptime rounds down: never better than it was.
- **Companies from their records**, not the dashboard's list (which leaves out a company whose probes all went quiet: found while testing); archived companies left out.
- **Once a day, restart-safe:** the local date it last went is stored, recorded before it is sent (as reminders, D-69); due from its hour until midnight, so a hub that was down then sends it when back that day. **Send digest now** doesn't count as the day's.
- **Delivery:** `Delivery::send_mail` through the channel's transport, recording its last delivery as any alert does; a disabled channel is skipped; a removed channel leaves the digest.
- **Not done here:** digests for one company (to send a customer), weekly digests, HTML mail, other channels.
- **Verified** by unit tests (a quiet day, a busy one, rounding, settings), storage conformance (settings and the day it went), integration tests (once a day at its local hour across a restart and the next day, the content with an open incident and uptime from stored results, send now, e-mail channels and real zones only, a removed channel) and the browser test of the Alerts page.

## D-73 · Probe health in each report, and the fleet view: how M4-02 was built
**Status:** Accepted (2026-10-09) · **Affects:** ROADMAP M4-02; protocol v8; proposal 0003 §4.2, 0004; `GET /api/probes`; the Probes page

- **The probe says how it is doing in every report** (protocol v8), as one optional `health` field appended to the signed report: `started_at` (its uptime), `queued_reports`, `clock_offset_secs`, `last_error`. Proposal 0003 §4.2 sketched `probe_health` with version, queue depth, uptime and clock; the version and the config state already travel in their own fields since v4/v5 (`probe_version`, `config_revision`, `config_rejected`), so they stay there and aren't repeated. Signed like the rest (FP-3): a hub only shows what the probe's key vouched for.
- **The clock offset is measured by the probe**, as its clock minus the `hub_time` of the hub's last signed answer, not worked out by the hub from the report's timestamp: a buffered report's timestamp is old by design, and the hub can't tell skew from delay. Whole seconds, round-trip time included; it is for spotting a drifting clock, and the threshold is a minute (`common::protocol::CLOCK_WARNING_SECS`, now shared by the probe's log warning and the fleet view), well under the 5 minutes at which the hub refuses reports.
- **`queued_reports` is the backlog when the report was made.** After an outage the probe delivers its backlog in order, newest last, so the newest report the hub has says how many waited before it; the next one says 0. Honest ("it just caught up 30 reports"), and needs no second message.
- **`last_error` is the newest problem of the past 24 hours**, kept in memory by the probe (`probe/src/health.rs`): a report it couldn't deliver (why), one the hub refused, a check task that failed, an enrollment it couldn't read, rotation keys it couldn't save. One problem, not a log: the probe's log file and `probe doctor` are for detail. A day so a problem at night is still there in the morning, and so the flag clears by itself. A restart forgets it, which `started_at` shows.
- **Stored as one JSON column** (`probes.health`, migration 0021), not one per field: the hub shows it and never queries it. A v7 report (no health) keeps the last health on record, like `pins` (D-61).
- **What needs attention is decided in one place**, `ProbeInfo::concerns()` in `hub/src/model.rs`, compiled into both the server and the WASM front end: never reported, not reporting (its liveness record expired), refused its checks, an update it didn't install, reports held back, clock more than a minute off, a problem in the past day. A revoked probe needs none. A pending config is not a concern: it reaches the probe with its next answer.
- **The fleet view is a page, `/probes`, built from `GET /api/probes`** (everyone sees the probes of their companies, as before), which now also works out each probe's config sync with the same `sync_state` as the probe page (it loads each probe's checks: one query per probe, fine for hundreds of probes on SQLite; a batched query can come if a fleet outgrows it). No new endpoint, so the scoping rules are the ones already tested. Times show as "running since" and "at", not "for 3 h" / "5 min ago": the page needs no clock of its own, and it reloads every 30 s.
- **Protocol: the hub accepts v7 and v8** (current and previous, ENGINEERING.md §2.7); v6 probes are refused and must be installed again once (pre-alpha, D-30). Rotation (D-61) is unchanged: it goes to v7 and later.
- **Verified** by unit tests (signed bytes of v8, the probe's health and its 24-hour problem, the outbox's failure reasons, `concerns()`, the page's helpers), a storage conformance test (health kept across a v7 report, `silent`), integration tests (health and sync through `GET /api/probes`, tampered health refused, v7 still accepted) and a browser test against the real probe (its health reported, the Probes page, its filters, the probe page's Health section).

## D-72 · Tickets through e-mail and webhooks; PSA connectors and SMS deferred
**Status:** Accepted (2026-10-08) · **Affects:** ROADMAP M3-10, M3-11; alert subjects and webhooks · **Decided by the owner**

- **No PSA connectors for now** (M3-10 deferred, owner's choice): HaloPSA, ConnectWise, Autotask and Syncro are paid, closed services, each with its own API, authentication and data model (customers, boards, ticket types), which we couldn't test without paid accounts and would have to keep up with. Most of them already make tickets from e-mail, and some take webhooks: the e-mail and signed webhook channels reach them today. The item stays on the roadmap, to revisit.
- **Instead, alerts that a PSA can follow** (owner's choice): every alert about an incident has the tag `[BLENTINEL #<incident>]` at the start of its subject (its first alert, changes, reminders, escalation, recovery), so a PSA's mail rules (or a mail client) put them in one ticket (or thread); the body says `Incident: #12 (open)`; webhooks carry `incident.ref` (the same tag) and `incident.state` (`open`, or `closed` for the recovery). Vendor-neutral, no code per PSA. A test alert keeps `[BLENTINEL]`, and the digest's subject is unchanged.
- **The tag is per hub** (the incident's ID): two hubs mailing one PSA can both send `#12`; the guide says to give each its own mailbox or rule. A hub-wide prefix can come if someone needs it.
- **SMS deferred too** (M3-11, owner's choice): it needs a paid gateway (Twilio and the like); ntfy (D-70) and Telegram (D-66) reach phones for free. To revisit.
- **Verified** by unit tests (the subject, the body line, the webhook's `ref` and `state`) and an integration test (a whole outage's alerts share one tag).

## D-74 · Managed probe rollout: how M4-01 was built
**Status:** Accepted (2026-10-09) · **Implements:** proposal 0012 §7 phase 2, D-41 · **Refines:** D-55's "not in phase 1" (window, per-company approval, canaries, pause, `allow_downgrade`); D-56's `upgrade` command kind; 0012 §5.2's probe-side `[update] window`

- **A rollout, not an approval.** The hub holds one rollout (`probe_rollout`, migration 0022, replacing D-55's `probe_update_approval`): the version, its **stage** (`canary` or `all`), the soak time, when every canary ran it, who promoted it, and a pause. Approving a release starts one: at the canary stage if any canary takes part, else straight to every probe (as phase 1 did).
- **Canaries are a flag on the probe**, ticked on the admin page. A canary takes part if it is active and its company follows the rollout (below): a revoked canary, or one of a company that is off or pinned, would never get the release and would hold the rollout forever.
- **"The canaries report healthy" means: every canary runs it, for the soak time.** A probe reports a new version only once that version has delivered a report, which is what confirms an update (D-55), so "runs it" already means "started, passed its self-test and reported". The soak time (60 minutes by default, 0 to a week, set when approving) catches what shows only later. The hub's 10-second sweep (`service::rollout::advance`) records when every canary first ran it and promotes once the soak has passed; a canary that stops running it clears that.
- **A canary that refuses it or rolls back pauses the rollout** (its `update_failed` names the version), with the canary and its reason, by the hub (no user), audited. Resuming while that failure stands pauses it again; the admin decides: take the probe out of the canaries, or **Roll out to all now**, which promotes even a paused rollout (and resumes it). The hub itself never promotes a paused one. Failures after promotion don't pause anything: each probe's watchdog rolls it back, and the page shows them. An alert on an automatic pause can come later.
- **Changes can't undo each other.** The hub's sweep and an admin can act at the same moment, so each change (`RolloutChange`) is one conditional `UPDATE` naming the version and the state it applies to (pause only if not paused, promote only from the canary stage, …), instead of a read-modify-write of the whole row. Approving or stopping replaces the row.
- **Per company: follow, off, or pinned, and a window** (`company_update_policy`, no row = follow at any time). A pinned company gets that release directly, whatever is rolled out, without canaries (the admin chose that release for it), and the release can't be removed while pinned. **The window is enforced by the hub, not the probe**: the hub only offers a release inside it, in local time at each probe's **site** (the site's time zone, as maintenance windows, D-65), past midnight if the end is earlier. So the probe needs no clock rules or settings (it has no config file, D-49), and changing a window takes effect at the next report. 0012 sketched `[update] window` on the probe; D-55 had already moved approval to the hub, and this follows. A download that starts inside the window may finish after it.
- **Signed rollbacks** (0012's `allow_downgrade`), as a separate signed file, `rollback.json`: `{version, from: [...], signed}`, signed with the release key by `blentinelmake release rollback --to X --from Y,Z`. Not a flag in the manifest, because the old release's manifest is already signed and on the hub, and re-signing it would make a second, different "0.4.1". A probe installs an older release only if the rollback is signed by a trusted release key, is for that version, and **names the probe's own version** in `from`: so a hacked hub can replay it only onto exactly the versions the signer allowed, never onto newer ones (every `from` must be newer than `version`). The signature covers `blentinel probe rollback v1\n` + the bytes (**domain separation**), so no manifest signature can pass for a rollback, nor the other way round. The hub checks it when it is added (alone, for a release it has, or with the release; a newer one replaces it), and offers an older release only to probes it allows, so the page shows "no signed rollback" instead of a refusal. It rides in the offer as `update.rollback`, which older probes ignore: no protocol bump.
- **Through the signed answer, not a command.** 0012 said phase 2 would replace polling with the command channel. Phase 1 already removed polling: the offer rides in the signed answer to every report (D-55), which is state (offered until installed or refused) rather than an order to repeat. An `upgrade` command (D-56) would add a second path with the same trust and no new power, so there is none; "update now" for one probe is a canary, or a pin.
- **One decision, shown and used.** `service::rollout::plan` (pure) decides what a probe gets or why it waits, and `check_release` whether the release may go to it; ingest uses them to make the offer, and the admin page to show every probe's **update status** (`UpdateStatus`, in the shared model): so the page says exactly why a probe waits.
- **Who:** all of it is `ManageData` (Admins), like approving (D-55), and audited: `rollout.pause`, `rollout.resume`, `rollout.promote`, `probe.canary`, `company.update_policy`.
- **Not done here:** a percentage of the fleet as a stage, more than one canary stage, alerts on an automatic pause, a window per site.
- **Verified** by unit tests (the plan for every status, windows past midnight, site time zones, the canary steps, which canaries count, rollbacks signed, domain separated and only to older versions, the probe's decision with and without one, the hub's store with a rollback alone, with its release or for another), storage conformance (the rollout's changes in order, canaries kept on enrolling again, policies, site zones), integration tests (canaries then everyone after the soak, no canaries, a failing canary pausing and an admin promoting, pause and resume, off, pinned and windows, a rollback uploaded alone and offered only to the versions it names, Admins only) and the browser test of the admin page.

## D-75 · Hub upgrades: how M4-04 was built
**Status:** Accepted (2026-10-09) · **Implements:** proposal 0012 §8, D-41's "never unattended" · **Refines:** 0012 §8's "the button next to the banner", D-57's release files

- **A command on the server, not a button.** The upgrade is `hub upgrade`, run as root (Administrator) on the server. 0012 §8 sketched a button beside the banner, but the service runs as the unprivileged `blentinel` user (D-52) and can't replace `/opt/blentinel/hub/hub`, by design: giving the web process that power would make a stolen admin session a way to run any program as root. So the page tells and the server does: it shows the version found and the exact command. That is also what "never unattended" (D-41) means here: an admin on the server decides.
- **The same release files as `hub.sh`** (D-57), plus `release.json` (`{format, version, released}`) listed in the signed `SHA256SUMS`, so the version is as trusted as the files and can be read without downloading a hub. The hub checks the signature against the release keys compiled into it (`common::hub_release::open_sums`, the raw Ed25519 signature `hub.sh` checks with `openssl`), and each file against `SHA256SUMS`. The source is `[upgrades] release_url` (GitHub's latest release by default; a URL or a folder on the server, like `BLENTINEL_RELEASE_URL`), or `--from`. A place on the server, so it stays in the config file (as `[backups] path`, D-60); the web UI never chooses where the hub downloads from.
- **Only newer.** `hub upgrade` refuses a release that isn't newer than the hub running: an older hub can't open a database a newer one migrated (D-52), and going back is `hub restore` with the old program, which the upgrade leaves as `hub.prev`.
- **The self-test runs the new program, on a copy.** `hub self-test` (a command of the new hub) prints its version first (which must be the release's), reads the config and key, copies the database (`backup_to`, consistent while the old hub runs), migrates the copy to its schema and runs the integrity check, then deletes the copy. A hub that can't start on this machine, or whose migrations fail on this data, is refused before anything changes: no backup, no stop.
- **Back up, switch, check, or put everything back.** A `-pre-upgrade` backup (D-60's kind, so the newest 3 are kept), then stop, rename the program to `hub.prev`, rename `hub.new` into place, start. Healthy means the service runs and the port answers within 90 s, and **stays** so for 15 s: systemd restarts a crashing hub, so "came up once" isn't enough. If not: stop, keep the new program as `hub.failed`, put `hub.prev` back, put the database back from the backup (`restore_database`: the one the new hub left goes to `failed-upgrade-<date>/`), give the service its files back, start the old hub and check it too; if even that fails, it says to run `hub doctor` and `hub restore <backup>`. The new hub's own start-up still backs up and migrates (D-60); the upgrade's backup is the one it can name and restore.
- **Tested without a service.** The steps that touch the machine (stop, start, healthy, self-test, file ownership) are a trait (`ops::hub_upgrade::Machine`); the real one uses `ops::platform`, the tests a fake, so the switch, a refused self-test, a rollback with the database put back, and a rollback that fails are all unit tests on real files and a real database.
- **The release's probes come with it** (0012 §4): after a successful upgrade, `probes.tar.gz` is added like `hub release add`, ready to approve (D-74). A failure there is a warning: the hub is already upgraded.
- **The notice is opt-in and only tells.** Off by default, because it contacts an outside server (FP-1 is about probes, but a hub that phones home unasked would surprise an MSP). On the admin page: **Check daily** (stored in `hub_upgrade_check`, migration 0023, audited `hub.upgrade_check`) and **Check now**. The hub's sweep checks when it is on and a day has passed since the last check; a failed check keeps the version found before and shows why. A newer version is shown only if its `SHA256SUMS` is signed by a trusted key, so nobody can make the page announce a fake release.
- **Windows:** the hub can upgrade itself the same way from a release that has a Windows hub, but `blentinelmake release hub` builds Linux (musl) hubs only, so on Windows `hub upgrade` says there is none, and the new `hub.exe install` remains the way (with D-60's backup before a database upgrade).
- **Not done here:** a Windows hub in releases, upgrading a hub run by hand (not as a service), a scheduled upgrade, an alert when a new version is out (the notice is on the admin page and in the log).
- **Verified** by unit tests (signed sums, plain file names, `release.json` against the sums, version comparison, when the check is due, the switch, a refused self-test changing nothing, a rollback restoring program and database, a failed rollback's advice), storage conformance (the check's settings and results), integration tests (the check off by default and daily once on, check now with a newer, same and forged release, Admins only, staging only a newer signed hub that matches its checksum, the self-test on a copy leaving the database untouched) and the browser test of the admin page (check now finds the newer release from a local folder, the daily check on and off). Not yet on a systemd host: the real service start, health check and rollback.

## D-76 · Commands within seconds: how M4-03 was built
**Status:** Accepted (2026-10-09; the design chosen with the owner) · **Implements:** proposal 0003 §4.3's long-poll · **Refines:** 0003's `GET /api/probe/wait`; D-56's "a command waits up to one report interval"

- **A separate command poll, not a faster report.** The owner asked whether a plain poll every 30 s (separate from reports, the interval set by the hub) wouldn't do instead of holding a request open. Both keep the report cycle as it is; they differ in latency (a held poll answers in about a second, a 30 s poll in 15 s on average), hub state (a held poll needs a connection per probe and a way to wake it; a poll needs none) and fragility (a proxy can cut a held request). The request rate is about the same. So we built **one request whose answer sets the pace**: the hub may answer at once ("ask again in 30 s": polling) or hold it ("ask again now": long-poll), or tell the probe to stop asking, and the probe can't tell a held answer from a slow one. Changing the policy is a hub change only, never a probe update.
- **The hub holds, up to 25 s** (`common::poll::HOLD_SECS`), under the 30 s after which many proxies drop a quiet connection, and answers as soon as a command is queued for that probe (`service::poll::CommandWaiters`: one waiter per probe, woken by `commands::queue`; a permit is kept if the command lands just before the hold starts). A newer poll from the same probe releases the older one, so a probe holds one at a time. Beyond 10,000 held polls the hub answers at once with "ask again in 30 s". Holding costs an idle connection and a timer per probe, cheap in tokio; the poll itself is a decrypt, a signature check and two small queries, about every 25 s per probe.
- **The probe falls back to polling.** Whatever goes wrong (no answer within 45 s, an error, an answer it can't trust) it asks again in 30 s (`FALLBACK_POLL_SECS`), the slowest it learns of commands while the hub is reachable; `404`/`405`/`403` (an older hub, a revoked probe) mean "ask again in an hour"; it never asks more than once a second, whatever an answer says. Commands still come with every report (D-56), so a poll that fails only makes them slower, and nothing about delivery or "at least once" changes.
- **Trusted like a report and its answer.** The poll is sealed for the hub and signed with the probe's key; the answer is signed with the hub's key, sealed for the probe and bound to the poll's SHA-256 (D-48). Both signatures are **domain separated** (`blentinel command poll v1\n`, `… answer v1\n`): the same keys sign reports and acks, and neither can pass for the other. A poll more than 5 minutes off the hub's clock is refused. A captured poll replayed within that window only marks the probe's waiting commands as sent, which they are again in its next answer; it can't change what the probe runs.
- **Carrying the commands, not just "report now".** The answer carries the commands themselves, so the probe runs them at once and reports once, with the results. A bare "report now" would reuse the report's answer but cost a round trip, and run the checks twice for "Run checks now". The probe hands commands from its poller task to the monitoring loop over a channel; the loop runs them like those from a report (the same memory of IDs it ran) and reports at once if there is anything to say.
- **`POST`, not `GET /api/probe/wait`** (0003's sketch): the request has a sealed body. A probe-facing route like `/api/report`, on both ports.
- **No protocol bump: still v8.** Reports and acks don't change. A v8 probe from before M4-03 just doesn't poll; a new probe that gets `404` from an older hub asks once an hour.
- **Not configurable yet.** The hold and the fallback are constants; the answer's `next_poll_secs` is where a setting (per hub, company or probe, e.g. "poll every N s" for sites behind strict proxies) would plug in, without touching probes.
- **Verified** by unit tests (poll and answer sealed, signed, bound and domain separated; stale, foreign and unsigned polls refused; holds that time out, wake, and release an older poll; the probe's pacing for every answer and failure), integration tests (a held poll answered within the hold when a command is queued, at once while one waits, held for the whole hold when none does; untrusted, stale, unknown and revoked polls refused; the route open only to probes) and a browser test against the real probe (a command queued right after a report is done within 3 s, which the next report, 5 s away, couldn't do).

## D-77 · The TLS certificate check: how M5-01 was built
**Status:** Accepted (2026-10-11) · **Implements:** proposal 0006 #1, D-64's "TLS expiry joins with M5-01" · **Challenges:** 0006's "30/7 days" (the built-in warning is 21) and its "weak chain?" (not looked at)

- **A check kind, `tls`:** `host`, `port`, and `verify` (default `true`, left out of the JSON then). The probe makes a real handshake (rustls, TLS 1.2 and 1.3, `host` as the server name) and reads the certificate the server presents. Its headline metric is `days_left` (decimals, to the certificate's last moment). The form takes an empty port as 443.
- **The handshake always finishes.** A normal client stops at a certificate it doesn't accept, and then there is nothing to read. The check's verifier runs rustls's own verification, **keeps** its verdict, and lets the handshake go on (the server must still prove it holds the key). So a refused certificate still reports its days, and the message says why it was refused.
- **What the probe decides, and what the hub does.** The probe says the check failed when the certificate has **expired** (always) or **wouldn't be accepted** by a client (unless `verify` is off): those are facts about the certificate, like a refused connection. *How few days are too few* is a threshold, so the hub decides it (0006 §1.5), and it can change without touching probes.
- **Trust: Mozilla's list, compiled in** (`webpki-roots`), never the machine's certificate store (FP-2: "no OS certificate store"). So a private authority installed on the probe's machine isn't trusted; `verify: false` is for those and for self-signed devices (dates only). The list gets newer with the probe (self-update, D-55).
- **Thresholds that fall.** `ThresholdMetric::Tls` is the first metric where less is worse: `ThresholdMetric::falls` and `reaches` are the one place that knows the direction (at or under, the warning limit above the down limit); judging, validation, the forms and the reasons use them. `Thresholds` gains `tls`; it is stored as JSON whose missing fields mean "nothing set", so there is **no migration**.
- **Built-in limits: warning at 21 days, down at 7** (0006 said 30/7). Let's Encrypt renews with about 30 days left, so a warning at 30 would fire on every healthy renewal; 21 means the renewal is late. Down at 7 follows the disk's 97 %: it still works, but someone must act now. Certificates that live a few days need their own limits.
- **No protocol change (still v8).** A probe from before this refuses a config with a `tls` check and keeps its previous one (D-49's rule: a config is applied whole or not at all); its page says so. Update probes first.
- **Not done here:** STARTTLS (SMTP 25/587, IMAP 143) and RDP's own negotiation; revocation; key and signature strength ("weak chain"); the expiry of intermediates; a server name different from the host connected to; client certificates; a private authority added in the UI; a list of upcoming expiries (0007's widget).
- **Seen while building it, not changed:** the probe's `http` checks verify HTTPS with reqwest's default (`rustls-platform-verifier`), which asks the **machine's** certificate store. That is at odds with FP-2's wording, and means an `http` and a `tls` check can disagree about a private authority. It belongs with M5-02 (HTTP expectations, with its "accept an invalid certificate" switch): use the same compiled-in list there.
- **Verified** by unit tests (a private authority and a local TLS server: trusted, untrusted with and without `verify`, another name, a port that is closed or speaks plain text; days counted to fixed dates; expired; the spec's JSON and validation; limits that fall, in the model, the service and the form), an integration test (a certificate going Up, Warning, Down as its days run out; the hub's own limits; inverted limits refused) and browser tests (the real probe reading the hub's certificate; the form; the admin page's limits). The browser test found the check dialog grown taller than the window, its Save button out of reach: dialogs now scroll.

## D-78 · HTTP expectations, and no OS certificate store: how M5-02 was built
**Status:** Accepted (2026-10-11) · **Implements:** proposal 0006 §2 (http), D-77's "seen while building it" · **Challenges:** 0006 §2's "max response time" as a setting and its basic auth (not built) · **Refines:** D-47's "without a pin, the system's trusted authorities"

- **Options beside the URL.** `CheckSpec::Http` gains `HttpOptions`, flattened into the same JSON object and each left out at its default: `method` (`get`, `head`), `status`, `contains`, `absent`, `follow_redirects`, `headers`, `verify`. So `{"kind": "http", "url": …}` means what it meant, existing checks and their config revisions don't change, and there is no migration or protocol bump (v8).
- **Status codes are text** (`"200, 301-308, 401"`), parsed by one function in `common` (`status_ranges`) that the form, the API and the probe share. It is what people type and read; a list of range objects would only move the parsing into the UI.
- **Text is looked for as written** (case matters, no patterns) in the first 1 MiB, read as UTF-8. `absent` is for the site that answers 200 with "Maintenance". A `HEAD` with text to look for is refused: there is no body.
- **"Max response time" is not a setting.** The hub's latency thresholds (D-64) already say it, at four levels, and change without a new probe config. A second place for the same rule would break "one source per fact".
- **No basic auth, and headers are not secret.** A check's spec is stored as written, shown by the API to everyone who sees the probe (Viewers, Clients), kept in backups and in the probe's `config.json`. A password there would be a leak by design. Credentials wait for sealed, write-only secrets on a check (as alert channels have, D-66), which the authenticated database check (0006 #8) needs too. Until then: expect the `401`. The form says so beside the headers.
- **Redirects and verifying belong to a client, not a request** (reqwest), so the probe keeps four clients, one per combination, made once (`Monitor::http_client`).
- **No OS certificate store any more (FP-2).** reqwest's rustls feature always links `rustls-platform-verifier`, so the probe now gives **every** client its own rustls configuration (`probe/src/tls.rs`): the hub's pins, Mozilla's list compiled in (`webpki-roots`), or, for a check with `verify` off, any certificate. That covers `http` checks and the one unpinned case of the hub link (a hub behind a proxy with a public certificate, D-47). The crate is still linked, but nothing calls it. A site or proxy whose certificate comes from a private authority installed on the probe's machine was trusted before and isn't now: `verify` off for a check; for a hub, its own certificate (pinned) or a public one.
- **Messages say why.** `HTTP 503 … (expected 200-299)`, `…, but the page doesn't contain "Sign in"`, and a connection error with its chain of causes (reqwest's own text stops at "error sending request").
- **Older probes ignore the new fields** (serde skips what it doesn't know) and judge the page the old way: unlike a new kind (D-77) the config isn't refused. Update probes first.
- **Not done here:** basic and bearer authentication, POST with a body, patterns (regular expressions), a match on answer headers, the certificate's days left from an `http` check (add a `tls` check), a timeout of the check's own, HTTP/2 (a preconfigured TLS client speaks HTTP/1.1), a private authority added in the UI.
- **Verified** by unit tests (the JSON with and without options; status lists; each refused option; a local web server: expected and unexpected status, HEAD, text present and absent, headers sent, redirects followed or not; a self-signed HTTPS server refused, then accepted with `verify` off; the form's fields to a spec and back), an integration test (the API keeps the options as sent and refuses bad ones) and browser tests (the real probe getting the redirect it expects from the hub's plain port; the form).

## D-79 · The file and folder check: how M5-03 was built
**Status:** Accepted (2026-10-11) · **Implements:** proposal 0006 #2 · **Refines:** 0006's "path + glob" (the pattern is for file names only)

- **A check kind, `file`:** `path` (a file, or a folder), and for a folder `pattern` (file names: `*`, `?`, any letter case), `recursive`, and `min_bytes`. The probe finds the **newest** matching file (by modification time) and reports `age` (hours, the headline), `size` (MB) and `files` (how many match).
- **Age is the hub's threshold, size is the check's setting.** How old is too old differs by backup plan and is worth setting for a whole company, so it is a threshold (`ThresholdMetric::FileAge`, hours, rising like usage and latency; `Thresholds.file_age`, no migration as in D-77). A check has thresholds on one metric only (D-64), and "at least 500 MB" describes the file expected, like an HTTP check's status codes (D-78): so `min_bytes` is part of the spec and the probe says the check failed.
- **Built-in limits: warning at 26 hours, down at 50.** Most file checks watch a nightly job: 26 hours is a run that is late, 50 is two missed. Other rhythms set their own.
- **The pattern matches names, not paths.** `*.bak` in a folder, with `recursive` for its subfolders, covers what people ask for; a glob across folders (`**/sql/*.bak`) would need a path grammar that differs between Windows and Linux, on a hub that doesn't know which one the probe runs. Paths must be full ones, in any system's form (`/…`, `C:\…`, `\\server\share`), checked by the one shared validation.
- **Native and read-only (FP-2):** `std::fs` only, metadata only. The walk runs on the blocking pool and **bounds itself** (200,000 entries, 16 levels, linked folders not followed): the 30 s limit on a check can't stop a blocking thread, so a scan that could run for minutes must end by its own count. It says when it stopped there.
- **What the hub learns.** A hub (or whoever controls it) can ask a probe whether a path exists and get the name, size and date of its newest file. That is less than the script check 0006 wants enabled locally, and it is what the feature is; it reads no content and changes nothing. Recorded here so it isn't a surprise: the newest file's name is in the result, visible to everyone who sees the probe.
- **No protocol change (still v8).** As with `tls` (D-77), an older probe refuses a config with this kind.
- **Not done here:** a threshold on the size, or on the number of files; total folder size; "newer than the previous run" and growth checks; a file that must *not* exist; reading a file's content (a status file's last line); credentials for a network share (Local System can't read most shares: documented).
- **Verified** by unit tests (the pattern matcher; a folder with files of set ages and sizes: the newest found, a pattern, subfolders only when asked, too small, none, a path that isn't there, one file; ages, sizes and a date in the future; the spec's JSON and its path rules for both systems; age limits in the model, service and form), an integration test (a file going Up, Warning, Down as it ages; a company's weekly limits) and browser tests (the real probe finding its own program; the form; the admin page's limit).

## D-80 · Ping with loss and jitter, and a TCP port to fall back on: how M5-04 was built
**Status:** Accepted (2026-10-11) · **Implements:** proposal 0006 §2 (ping) · **Refines:** D-64's "one metric per check kind" (a kind can be judged by a second one, from the levels above the check)

- **Several echoes.** `CheckSpec::Ping` gains `count` (1 to 20, default 4, left out of the JSON at 4) and `tcp_port`. The probe sends them 200 ms apart (a burst is what devices that limit ICMP drop first, and that would read as loss), each with 1 s to be answered: at most 24 s, inside a check's 30. The result's latency is the **average**; its metrics are `loss` (%, the headline), `latency_max` and `jitter` (the mean difference between one answer's time and the one before).
- **Up means at least one answer.** One lost echo isn't an outage; no answer at all is. Everything between is a number for the hub to judge.
- **Loss is a second thing a ping is judged by.** Reporting loss without being able to alert on it would be half the feature. `ThresholdMetric::Loss` joins the levels (hub, company, site; built-in: warning at 50 %, down off), and `ThresholdMetric::also_of` names what a kind is judged by beside its own metric. `Levels::judge` judges both and keeps the worse. A check's own limits stay one pair, for its own metric (latency): a second pair on the check would need a schema and a form for something a site-wide limit says better. With 4 echoes the steps are 25 %, so 50 % means two lost; 25 % happens on any Wi-Fi and flap suppression (D-63) would hide most of it anyway.
- **The TCP port is a fallback, not a second check.** Tried only when no echo is answered, or when the probe may not send any. Then the check is up if the port opens, with the connection's time as latency and **no loss metric** (there is nothing measured to judge). It answers "is the host there?" for firewalls and Windows machines that drop pings, which is what 0006 asked. The port has no default: nothing listens everywhere.
- **Fixed on the way.** The probe made one IPv4 ICMP client with `expect`: without the permission it **didn't start**, and an IPv6 address could never answer. Now one client per IP version, each optional (`Monitor::icmp_client`); a missing one is logged at startup and told by each check. Every ping used identifier 0 and sequence 0, so two checks of one host could take each other's answer: each run now has a random identifier.
- **No protocol change (v8).** An older probe ignores the new fields and sends one echo; without a `loss` metric its results are judged by latency alone.
- **A signature bug found and fixed: numbers must survive the wire exactly.** The probe signs the report's canonical JSON; the hub parses the report and writes it out again to check (D-18, `StatusReport::to_signable`). That holds only if every value comes back unchanged, and `serde_json` reads numbers exactly only with its `float_roundtrip` feature: without it about 7 % of long fractions come back one bit off, the bytes differ, and the hub answers `bad_signature`, which the probe treats as permanent (D-29) and drops. It was there since metrics were (M2-04), rare with usage percentages; averages and jitter made 3 of 8 reports fail in the browser tests. The feature is now on for the whole workspace (root `Cargo.toml`, with a warning not to drop it), and a test in `common/src/models.rs` sends 20,000 awkward values through. The same holds for the hub's signed answers. Checking the signature against the bytes received, not against a second serialisation, would remove the dependence altogether: a protocol change, for when the protocol next changes.
- **Not done here:** limits on loss or jitter on a check itself; a threshold on jitter; host names (a ping still takes an IP address: DNS is M5-07); the size, interval and timeout of echoes as settings; TTL and route.
- **Verified** by unit tests (runs of answers and losses: average, slowest, jitter, loss, the messages, one echo, none; the TCP fallback open and closed; this machine by echo or by port, whichever the system allows; the spec's JSON and limits; the form), tests of the judging (latency and loss, the worse counts, a site's limits) with an integration test through the API, and browser tests (the real probe reaching its machine; the form; the admin page's limit; and, by failing, the signature bug above).

## D-81 · Every disk in one check: how M5-05 was built
**Status:** Accepted (2026-10-11) · **Implements:** proposal 0006 §2 (local_disk: auto-discover all fixed volumes, used %, free GB, ignore-list) · **Refines:** 0006's wording (a new kind beside `disk`, not a mode of it)

- **A check kind, `disks`,** with one setting, `ignore`. The probe lists the machine's volumes each time it runs (it already refreshes them once per cycle) and keeps the fixed ones: with a size, not removable, not read-only, not of a file system that is an image, memory or another machine's (`squashfs`, `iso9660`, `tmpfs`, `nfs`, `cifs`, …), not ignored; and of several mounts of one device (bind mounts, subvolumes, a container host's layers) the shortest path.
- **One check, one result: the fullest disk.** Results, alert state, silences and limits belong to a check's ID (D-15, D-49), so a check can't return a result per disk without the hub creating checks by itself. So the headline `usage` is the fullest disk's, judged by the existing disk thresholds (`ThresholdMetric::Disk`: nothing new on the hub), with that disk's `free` GB and the count (`volumes`); the message lists every disk, fullest first (six named, the rest counted).
- **The reason names the disk.** Where a threshold decides, the UI and the alerts show the hub's reason instead of the probe's message (D-64), and "disk usage 92% is over the warning limit" doesn't say which disk. For this kind `Levels::judge` appends the probe's message to the reason. Only for this kind: for one disk it would say the same thing twice.
- **What one status costs,** said in the reference: while one disk is over its limit, a second one filling up changes the message but sends no new alert; one silence and one pair of limits cover all. A disk that needs its own gets a `disk` check (kept as it was). The other design, the hub making one check per disk the probe reports, gives each disk its alert but needs the probe to report its inventory and the hub to create, rename and retire checks on its own: that is M5-12 (host inventory → suggested checks), where an admin accepts them.
- **A new kind, not `disk` with a flag:** `{"kind": "disk"}` without a path already means the system disk, and an older probe would run a flagged `disk` as that, silently watching one disk where the hub says all. A kind it doesn't know it refuses, and its page says so (D-49, D-77).
- **The ignore list uses the file check's name patterns** (`*`, `?`, any letter case, D-79), and `E:` names `E:\` too.
- **Not done here:** a result or an alert per disk (above); limits per disk within the check; free space in GB as a threshold (a 4 TB disk at 95 % has 200 GB left: the percent limits can be set on the check); inodes; disk health (SMART, M5-16); the kind's own built-in limits.
- **Verified** by unit tests (a set of volumes as Linux and Windows list them: removable, read-only, a snap, memory, a network share, a second mount, container layers, an empty one; the ignore list by pattern and however a drive is written; the fullest first, the metrics, more than six, none; this machine's real disks), a test of the judging (the reason carries the list; one disk's doesn't) with an integration test through ingest, the form's test, and a browser test of the form.
