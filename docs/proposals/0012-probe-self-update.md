# 0012 · Probe Self-Update (and Hub Upgrades)

| | |
|---|---|
| **Status** | Accepted (owner, 2026-09-28) · **Important** |
| **Milestone** | Phase 1 in **M2** (M2-15); phase 2 and hub upgrades in M4 (M4-01, M4-04) |
| **Priority** | **P0** (probe, phase 1) |
| **Depends on** | `blentinelmake release sign` (§4; no CI needed) · hub upgrades: M2-07 (single-file hub), M2-08 (`hub backup`) · phase 2: 0003 (command channel) |
| **Supersedes** | the self-update sketch in 0004 §5.4 |
| **Decisions** | D-40, D-41 |

## 1. Problem

Every probe runs inside a client's network, often on a machine nobody at the MSP logs into. Today an upgrade means reaching each machine and running the installer. With tens or hundreds of probes, that doesn't happen, so fleets drift onto old versions. Every new check type (M5) and every security fix needs an upgrade.

**Goal:** when the admin approves a version on the hub, probes notice it, download it **from their hub**, check it's genuine, install it and restart, with nobody touching the client machines. A version that doesn't work rolls back on its own.

## 2. Decisions this design rests on (D-41)

| | |
|---|---|
| **Probes update only from their hub** | Keeps probes in sync with the hub's version and protocol. Client networks often allow only the connection to the hub (FP-1), so no extra firewall rules are needed. One download path, one configuration. |
| **Whoever builds, signs** | Today the admin builds the hub and the probes (the hub certificate is compiled into the probe, D-01), so the admin signs releases with **their own key**. Official Blentinel releases, once they exist, are signed with the Blentinel key; the mechanism is the same. |
| **The private signing key never lives on the hub** | A hub holding the key could sign anything: a compromised hub would become a compromised fleet in every client network. The key stays on the build machine (or a hardware token). The hub stores and serves **already-signed** files and can't create new ones. A hacked hub can at worst *withhold* updates. |
| **Off by default, switched at the hub** | The hub offers no update until the admin approves a version. Probes follow the hub, so enabling updates never means editing client machines. Each probe keeps a local emergency brake (`[update] enabled = false`). |
| **The hub is never updated unattended** | The hub may *check* for a new version and *tell* the admin (opt-in). Upgrading is always admin-triggered, with a database backup and rollback (§8). |

## 3. Principles check

| Principle | How this design keeps it |
|---|---|
| **FP-1 Push-only** | The probe *asks* its hub whether there's a new version and *downloads* it. Nothing ever connects to the probe. |
| **FP-2 Zero-dependency probe** | The updater is part of the probe binary: no separate updater, no scripts, no package manager. It uses the probe's existing connection to the hub (rustls, pinned certificate). |
| **FP-3 Zero-trust** | A binary is installed only if it's **signed with a release key the probe trusts** (compiled in; later pinned at enrollment, D-01). The hub and the network are not trusted for this. |
| **FP-4 Resilient probe** | **Self-test before the switch**, **watchdog after**: a bad version rolls back automatically. Monitoring keeps running while an update downloads. |
| **PP-2 Trivial to manage** | One place to decide (the hub), and results visible there. |

## 4. Releases and signing

- **Release key:** an Ed25519 key pair. `blentinelmake release keygen` creates it on the build machine. The **private** key stays there (or on a hardware token); the **public** key is compiled into every probe built there, next to the hub certificate.
- **Signing:** `blentinelmake release sign` writes, for the built probe artifacts, `manifest.json` and `manifest.json.sig` (an Ed25519 signature of the manifest). The manifest lists every binary's SHA-256, so one signature covers them all.
- **Publishing to the hub:** the admin uploads the signed release (manifest + signature + binaries) to the hub in the UI, or with `hub release add <folder>`. The hub checks the signature against the same public key before accepting it, so a wrong upload is caught early. The probe still checks for itself.
- **Hub releases carry their probes:** `blentinelmake hub publish` can include the signed probe binaries for every target. After an upgrade the hub offers the probes the version that matches it.
- **Key rotation:** the probe trusts **a list** of public keys. A release signed with the current key can introduce a new one. A compromised key is revoked by shipping, via a still-trusted key, a release that drops it.
- **Official releases (later):** the same format, signed with the Blentinel key, published on the Blentinel site (D-23). A hub can then fetch them (§8) instead of the admin building its own. Nothing changes for probes: they still download from their hub.

```json
{
  "version": "0.4.2",
  "released": "2026-11-02T10:00:00Z",
  "min_protocol": 2,
  "artifacts": [
    { "target": "x86_64-unknown-linux-musl", "file": "probe-0.4.2-x86_64-linux", "sha256": "…", "size": 7792504 },
    { "target": "x86_64-pc-windows-msvc",   "file": "probe-0.4.2-x86_64-windows.exe", "sha256": "…", "size": 8123392 }
  ]
}
```

## 5. How a probe updates itself

### 5.1 Discovering an approved version

The hub serves the approved release under `/api/update/probe/` (`manifest.json`, `manifest.json.sig`, the binaries). Nothing is served until the admin approves a version (off by default). The probe checks there at startup, then every `check_interval` (default 6 h, with random jitter so a fleet doesn't ask at the same second). Phase 2 replaces polling: the hub answers a report with "version X is approved for you" (0003).

### 5.2 Deciding to update

The probe updates only if **all** of these hold:

1. Updates aren't disabled locally (`[update] enabled = false` is the emergency brake).
2. The manifest signature verifies against a key the probe trusts.
3. `version` is **newer** than its own. Downgrades are refused, which blocks replaying an old, vulnerable release. A rollback the admin wants is a signed manifest with `"allow_downgrade": true`.
4. There's an artifact for its own target (compiled in: the Rust target triple).
5. The update window allows it: `[update] window = "02:00-05:00"` (optional; phase 2: set per company on the hub).
6. `min_protocol` is one the probe's hub speaks. It always is when the release comes from that hub, but the check is cheap.

### 5.3 Downloading and checking

- Download to `probe.new` **in the probe's own folder**, so the switch is an atomic rename on the same file system. It uses the probe's pinned connection to the hub, is limited to the size stated in the manifest, and has a timeout. Monitoring continues meanwhile.
- Check the **SHA-256** against the signed manifest. On a mismatch, delete the file and log an error. After 3 failures for the same version, stop trying until a newer version is approved.

### 5.4 Self-test before switching

The probe runs `probe.new --self-test` as a separate process, **without replacing anything yet**. The new version must:

- start, and print its version (it must equal the manifest's);
- load the current config and identity (`identity.key`);
- complete a hub handshake (or verify the pinned hub key);
- exit 0 within 60 s.

If any step fails, the update is abandoned and the running version keeps going. This catches a release that can't even start, before anything is replaced.

### 5.5 Switching

1. Rename `probe` → `probe.prev` and `probe.new` → `probe`. Both Linux and Windows allow renaming a running executable; Windows forbids only overwriting it.
2. Write `update.state` (`{ "from": "0.4.1", "to": "0.4.2", "since": … }`) next to the binary.
3. Restart:
   - **systemd:** the probe exits with code 75; the unit's `Restart=always` (plus `RestartForceExitStatus=75` from the installer) starts the new binary.
   - **Windows service:** the probe stops with a service-specific error; the recovery action ("restart on failure", set by `install_probe_service.ps1`) starts the new binary.
   - **Run by hand:** the probe starts the new binary itself and exits.

### 5.6 Watchdog and rollback

While `update.state` exists:

- **Crash loop:** every start increments a counter, as the very first thing `main` does. After 3 starts without confirmation, the probe restores `probe.prev` and restarts.
- **Can't report:** the new version must **deliver a report** within 15 minutes (or 3 intervals, whichever is longer), or it restores `probe.prev`. The old version had a working connection when it updated, so a missing report points to the update, not to the hub.
- **Success:** the first delivered report confirms the update. `update.state` is removed; `probe.prev` is kept as a manual fallback until the next update.
- A version that was rolled back is remembered (`update.rejected`) and not tried again.

### 5.7 What the hub sees

- Every report carries the probe's version: a new signed field `probe_version`, protocol v3, which the hub accepts alongside v2 (ENGINEERING §2.7).
- The next report after a switch carries the outcome (`updated from 0.4.1`, or `rolled back from 0.4.2: <reason>`).
- The admin page shows the approved version, which probes run which version, and recent failures.

## 6. Configuration

On the **hub** (reloaded like every setting, D-38; in the UI with M2):

```toml
[updates]
probe_version = ""      # the approved probe version; empty: no updates offered (default)
```

On the **probe:**

```toml
[update]
enabled = true          # local emergency brake: false refuses updates even if the hub offers one
check_interval = "6h"
window = ""             # e.g. "02:00-05:00" (probe local time); empty: any time
```

`probe --check-update` shows what the probe would do; `probe --update-now` does it immediately (testing, scripted rollouts).

## 7. Phases

| Phase | What | Milestone |
|---|---|---|
| **1 · Probe updates from its hub** | `blentinelmake release keygen/sign`, public key compiled into the probe, signed releases uploaded to the hub (`hub release add`), hub serves the approved version, probe check/download/SHA-256/self-test/switch (systemd, Windows service, by hand)/watchdog/rollback, `probe_version` in reports (protocol v3), version view on the admin page | **M2** (M2-15, P0) |
| **2 · Managed rollout** | Approved version and update window per company, pause, signed rollback, canary group first then the rest when the canaries report healthy, via the command channel (0003) instead of polling | M4 (M4-01) |
| **Hub upgrades** | §8 | M4 (M4-04), after M2-07 and M2-08 |

## 8. Hub upgrades (never unattended)

The hub holds all the data, the keys and the logins, so it is **never updated automatically**. Rolling it back also means restoring its database, not just swapping a file.

1. **Check and notify (opt-in):** if enabled, the hub checks a release URL for a newer version (later the official Blentinel site) and shows a banner on the admin page. Off by default, because it means the hub contacts an outside server.
2. **Admin-triggered upgrade:** `hub upgrade` (or the button next to the banner). It:
   - downloads the release;
   - checks its signature (the release key compiled into the hub);
   - makes a **database backup** (M2-08);
   - checks the new binary with `--self-test`: it opens a *copy* of the database and applies the migrations there;
   - switches;
   - rolls back binary **and** database if the new hub doesn't come up healthy.
3. **The release brings its probes:** after the upgrade, the new probe version is ready to approve (§4).

Prerequisites: the single-file hub (M2-07) makes the switch a single rename, as for the probe; `hub backup`/`restore` (M2-08) makes the rollback possible.

## 9. Tests

- **Unit:**
  - manifest parsing;
  - signature checks: valid, unknown key, tampered manifest, tampered binary hash;
  - downgrade refused;
  - local brake, window logic;
  - `update.state` transitions (pending → confirmed / rolled back, crash counter);
  - the hub refuses an upload with a bad signature.
- **Integration:** the hub serves a real signed release, and a probe updates from it. Tampered, oversized and slow artifacts are refused without changing anything.
- **End to end on Linux (systemd container) and Windows (service):**
  - v1 → v2 succeeds and v2 reports;
  - a v2 that fails `--self-test` is never switched to;
  - a v2 that crashes at startup rolls back after 3 starts;
  - a v2 that can't report rolls back after the timeout;
  - `identity.key`, config and buffered reports survive every path.
- **Hub upgrade:** upgrade with migrations succeeds; a broken new hub rolls back binary and database.
