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
**Status:** Partly done: probes, companies and sites are in the database and managed in the UI (M1-03, M2-01; `[[probes]]` removed, not imported: D-30, D-47). `[alerts]` is still TOML · **Affects:** `[[probes]]`, `[alerts]`, hub hot-reload · **Roadmap:** M1/M2

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
**Status:** Accepted (implemented in M2-04, see D-49) · **Roadmap:** M2 (with 0003)

**Challenge:** identity built from mutable text means renaming a check or changing its target loses its history, silences, and alert state, and a `:` inside a name breaks parsing.

**Decision:** checks become hub records with a UUID `CheckId` (proposal 0003). Results, silences, incidents, and alert state reference `CheckId`.

## D-16 · Binary Up/Down status → Up / Warning / Down / Unknown
**Status:** Proposed · **Roadmap:** M3 · proposal 0005

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
**Status:** Accepted (2026-09-28, owner) · **Design:** [0012](proposals/0012-probe-self-update.md) · **Supersedes:** the self-update sketch in 0004 §5.4

Probes must be able to detect a new version, download it from a URL and update themselves (owner request, marked important). The standalone part (signed manifest, download, self-test, switch, rollback) needs neither the command channel (0003) nor CI, so it moves from M4 to **M2 at P0**. Hub-managed and staged rollouts stay in M4. Non-negotiable: a probe installs only binaries signed with the release key compiled into it, never downgrades on its own, and rolls back a version that can't start or report.

## D-41 · Updates come from the hub; the signing key never lives on the hub
**Status:** Accepted (2026-09-28, owner) · **Refines:** D-40 · **Design:** [0012](proposals/0012-probe-self-update.md)

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
**Status:** Accepted (implemented 2026-09-28; the owner chose removing `[[probes]]`, the hub assigning company and site at "Add probe", and single-use 24 h codes by default) · **Implements:** proposal 0002 §4.2–4.4, D-01, D-02, D-17 · **Supersedes:** the `[[probes]]` whitelist (D-04, for probes), auto-registration (D-44)

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
