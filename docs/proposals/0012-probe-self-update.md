# 0012 · Probe Self-Update

| | |
|---|---|
| **Status** | Proposed · **Important** (owner, 2026-09-28) |
| **Milestone** | Phase 1 in **M2** (pulled forward from M4); phases 2–3 in M4 |
| **Priority** | **P0** |
| **Depends on** | Signed releases (§4, can be done with `blentinelmake` before CI, M0-13) · phase 3: 0003 (command channel) |
| **Supersedes** | the self-update sketch in 0004 §5.4 (M4-01) |
| **Decisions** | D-40 |

## 1. Problem

Every probe runs inside a client's network, often on a machine nobody at the MSP logs into. Today an upgrade means reaching each machine and running the installer. With tens or hundreds of probes, that doesn't happen, so fleets drift onto old versions. Every new check type (M5) and every security fix needs an upgrade.

**Goal:** a probe notices that a newer version exists, downloads it from a URL, checks it's genuine, installs it, and restarts, with no one touching the machine. If the new version doesn't work, it rolls back on its own.

## 2. Principles check

| Principle | How this design keeps it |
|---|---|
| **FP-1 Push-only** | The probe *asks* whether there's a new version and *downloads* it. Nothing ever connects to the probe. The hub may say "update to X" only in its answer to the probe's own report (phase 3). |
| **FP-2 Zero-dependency probe** | The updater is part of the probe binary: no separate updater program, no scripts, no package manager. Downloading uses the probe's existing HTTP client (rustls). |
| **FP-3 Zero-trust** | A binary is installed only if it's **signed with the Blentinel release key**, which is compiled into the probe. HTTPS, the download server, and even the hub are not trusted for this: a compromised server can withhold updates, but can't make a probe run its code. |
| **FP-4 Resilient probe** | The new version runs a **self-test before the switch**, and a **watchdog** after it. A bad version rolls back to the previous binary automatically. Monitoring keeps running while an update downloads. |
| **PP-2 Trivial to manage** | On by default for the stable channel, with sensible limits. The admin sees versions and update results in the hub (phase 2). |

## 3. How a probe updates itself

### 3.1 Discovering a new version

The probe checks an **update manifest**, a small JSON file, at a URL:

- **Default:** the official release server (e.g. `https://releases.blentinel…/probe/stable/manifest.json`, or GitHub Releases).
- **Configurable:** `[update] url = …`, e.g. the MSP's own server or **the hub itself** (phase 2). This matters because in many client networks a probe can reach only its hub.

It checks at startup, then every `check_interval` (default 6 h, with random jitter so a fleet doesn't hit the server at the same second).

```json
{
  "version": "0.4.2",
  "channel": "stable",
  "released": "2026-11-02T10:00:00Z",
  "min_protocol": 2,
  "artifacts": [
    { "target": "x86_64-unknown-linux-musl", "url": "https://…/probe-0.4.2-x86_64-linux", "sha256": "…", "size": 7792504 },
    { "target": "x86_64-pc-windows-msvc",   "url": "https://…/probe-0.4.2-x86_64-windows.exe", "sha256": "…", "size": 8123392 }
  ]
}
```

Next to it, `manifest.json.sig`: an **Ed25519 signature of the manifest** with the release key. The manifest names each binary's SHA-256, so one signature covers every file.

### 3.2 Deciding to update

The probe updates only if **all** of these hold:

1. The manifest signature verifies against a trusted release key (§4).
2. `version` is **newer** than its own. Downgrades are refused, which blocks replaying an old, vulnerable release. A rollback the admin wants is an explicitly signed manifest with `"allow_downgrade": true` (phase 2).
3. There's an artifact for its own target (compiled in: the Rust target triple).
4. The update window allows it: `[update] window = "02:00-05:00"` (optional; default: any time).
5. The hub it reports to speaks the new version's protocol (`min_protocol`), so an update can't cut the probe off from an older hub.

### 3.3 Downloading and checking

- Download to `probe.new` **in the probe's own folder**, so the final switch is an atomic rename on the same file system. Limits: the size stated in the manifest, and a timeout. Monitoring and reporting continue meanwhile.
- Check the **SHA-256** against the signed manifest. On any mismatch, delete the file and log an error. It is retried at the next check, and after 3 failures for the same version, the probe stops trying until a newer version appears.

### 3.4 Self-test before switching

The probe runs the new binary as `probe.new --self-test`. The new version must, **as a separate process, without replacing anything yet**:

- start, and print its version (it must equal the manifest's);
- load the current config and identity (`identity.key`), which proves it can read the existing state;
- complete a hub handshake (or verify the pinned hub key);
- exit 0 within 60 s.

If any step fails, the update is abandoned and the running version keeps going. This catches the most dangerous class of bad release, one that can't even start, before anything is replaced.

### 3.5 Switching

1. Rename `probe` → `probe.prev` and `probe.new` → `probe`. Both Linux and Windows allow renaming a running executable; Windows forbids only overwriting it.
2. Write `update.state` (`{ "from": "0.4.1", "to": "0.4.2", "since": … }`) next to the binary.
3. Restart:
   - **systemd:** the probe exits with code 75 (`EX_TEMPFAIL`); `Restart=always` starts the new binary. The installer's unit gets `RestartForceExitStatus=75`, so this also works if the restart policy changes.
   - **Windows service:** the probe stops with a service-specific error code; the recovery action ("restart on failure", already set by `install_probe_service.ps1`) starts the new binary.
   - **Run by hand (no service manager):** the probe starts the new binary itself and exits.

### 3.6 Watchdog and rollback

On startup the probe reads `update.state`. While an update is pending:

- **Crash loop:** every start increments a counter in `update.state`. After 3 starts without confirmation, the binary restores `probe.prev` and restarts. This check is the first thing `main` does, before anything that could fail.
- **Can't report:** the new version must **deliver a report** within 15 minutes (or 3 intervals, whichever is longer). If it can't, and the old version could, it restores `probe.prev`, restarts, and logs why. So that a hub outage isn't blamed on the update, a handshake failure counts only if the old version had a working connection when it updated.
- **Success:** the first delivered report confirms the update. `update.state` is removed; `probe.prev` is kept until the next successful update, as a manual fallback.
- A version that was rolled back is remembered (`update.rejected`) and not tried again.

### 3.7 What the hub sees

- Every report carries the probe's version. A new signed field, `probe_version`, is protocol v3, which the hub accepts alongside v2 (ENGINEERING §2.7).
- The update outcome is reported in the next report after the switch (`updated from 0.4.1`, `rolled back from 0.4.2: <reason>`).
- Phase 2 UI: version distribution per company, probes on unsupported versions, recent update failures.

## 4. Release signing

- **One Ed25519 release key** (vendor key, not per hub; no conflict with D-01). The private key stays offline: on the maintainer's machine or a hardware token, later a CI secret.
- `blentinelmake release sign` writes `manifest.json` and `manifest.json.sig` for built artifacts, so **releases can be signed before CI exists** (M0-13 is on hold).
- The probe embeds **a list** of trusted public keys, so the key can be **rotated**: a release signed with the old key introduces the new one. A compromised key is revoked by shipping (via the still-trusted key) a release that drops it.
- Self-built probes (custom certificate, D-01 today) either sign with their own key (`BLENTINEL_RELEASE_KEY` at build time) or have updates disabled.

## 5. Configuration

```toml
[update]
enabled = true          # default true for official builds; false for custom builds without a release key
channel = "stable"      # stable | beta
url = ""                # empty: the official release server for `channel`; or your own server, or the hub (phase 2)
check_interval = "6h"
window = ""             # e.g. "02:00-05:00" (probe local time); empty: any time
```

All of it hot-reloads (D-38). `probe --check-update` shows what the probe would do. `probe --update-now` does it immediately (for testing, and for scripted rollouts).

## 6. Phases

| Phase | What | Milestone |
|---|---|---|
| **1 · Standalone self-update** | Manifest + signature, release key and `blentinelmake release sign`, `[update]` config, download, SHA-256, self-test, switch (systemd, Windows service, by hand), watchdog and rollback, `probe_version` in reports (protocol v3), `--check-update` / `--update-now` | **M2** (P0) |
| **2 · Hub-managed updates** | The hub mirrors releases (fetched from the release server or uploaded by the admin) and serves the manifest, so probes that can only reach the hub update too. UI: versions per company, failures, target version per company, update window, pause, signed rollback manifest | M4 |
| **3 · Staged rollout** | Through the command channel (0003): the hub tells probes, in its answer to their reports, which version to run. Canary group first, then the rest when the canaries report healthy; automatic pause on failures | M4 |

## 7. Tests

- **Unit:** manifest parsing, signature checks (valid, wrong key, tampered manifest, tampered binary hash), downgrade refused, window logic, `update.state` transitions (pending → confirmed / rolled back, crash counter).
- **Integration:** a fake update server (as in `hub_link` tests) serving good, tampered, oversized and slow artifacts. The probe must update, or refuse without changing anything.
- **End to end on Linux (systemd container) and Windows (service):**
  - v1 → v2 succeeds and v2 reports;
  - v2 that fails `--self-test` is never switched to;
  - v2 that crashes at startup rolls back after 3 starts;
  - v2 that can't report rolls back after the timeout;
  - `identity.key`, config and buffered reports survive every path.

## 8. Open questions for the owner

1. **Where are official releases hosted?** GitHub Releases (free, simple) or a Blentinel server (needed anyway for the product site, D-23)?
2. **Who holds the release key,** and where (file on your machine, hardware token)?
3. **Default on or off?** Proposed: on for official builds (stable channel, any time). The alternative is off until the admin enables it.
4. **Should the hub also self-update** in the same way? It's not part of this proposal. The hub runs where the admin has access anyway, and `hub upgrade` is planned (0004).
