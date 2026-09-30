# Installing a probe

A probe is one self-contained binary that runs inside a client network and pushes reports to the hub. It needs **outbound** access to the hub's URL and nothing inbound. Every hub uses the same probe binary: which hub it trusts comes from its **enrollment code**, not from the build.

> **Pre-alpha.** Build from source ([CONTRIBUTING.md](../../CONTRIBUTING.md)). One-command installation with per-OS one-liners is planned (ROADMAP M2-09).

## 1. Build for the target machine

```sh
./target/release/blentinelmake probe publish --target x86_64-unknown-linux-musl   # Linux x86_64
./target/release/blentinelmake probe publish --target aarch64-unknown-linux-musl  # Raspberry Pi 4/5
./target/release/blentinelmake probe publish --target x86_64-pc-windows-msvc      # Windows
```

Use the **musl** targets on Linux: the binary is fully static and runs on any distribution. Copy the resulting `app/` folder to the probe machine. Everything the probe creates (identity, enrollment, checks, buffer, log) stays in its own folder. There is no config file to edit.

## 2. Get an enrollment code

In the hub's web UI: **Companies** → the company → **Add probe**. If the company or its site doesn't exist yet, add it there first (Admins add companies; see [guide/companies-sites.md](../guide/companies-sites.md)).

- **Site:** where the probe is.
- **Probe name:** optional; empty means the machine's hostname.
- **How many probes** may use the code (1 by default; more for rolling out several) and **how long** it's valid (24 hours by default).
- **Hub address, as probes reach it:** pre-filled with the address you're using in the browser (`https://<hub>:3443`). Change it if the probe reaches the hub by another name or address.

The hub shows the code **once**, starting with `BLT1-`. It contains the hub's address, its key and the fingerprint of its certificate, so the probe trusts that hub from the first packet.

## 3. Enroll

On the probe machine, in the probe's folder:

```sh
./probe enroll BLT1-…
```

```
Enrolled with https://hub.example.com:3443
  Probe   : FRONTDESK-PC
  Company : ACME Ltd
  Site    : Headquarters
  Saved   : /opt/blentinel-probe/enrollment.json
```

The probe creates its identity (`identity.key`) if needed, sends its enrollment to the hub (signed with that key, encrypted for the hub's key, over HTTPS pinned to the hub's certificate) and saves `enrollment.json`. The company and site are the hub's decision: the probe can't report for any other.

## 4. Start it, and give it checks on the hub

```sh
./probe
```

The probe reports to the hub right away and appears on its company's page and on the dashboard. A probe started before it's enrolled waits and says so, then starts by itself once `enrollment.json` appears.

**What it checks, and how often, is set on the hub**, not on the probe machine: each check is a record on the hub, and the hub sends the probe its checks and report interval in its signed answer to a report. The probe runs a new set at once and keeps it in `config.json`, so after a restart, or while the hub is unreachable, it runs the last checks it received. A new probe checks nothing until it has checks, and reports every 60 seconds.

Until the Checks page arrives (ROADMAP M2-05), add checks with the hub's API while logged in, for example:

```sh
# POST /api/probes/<probe id>/checks   (the probe ID is on its company's page and in GET /api/probes)
{ "name": "Core switch", "spec": { "kind": "ping", "host": "192.0.2.1" } }
```

The kinds and their settings are in [reference/checks.md](../reference/checks.md); the endpoints in [reference/api.md](../reference/api.md#checks). The report interval is set with the probe's name and site (`POST /api/probes/<id>`, `interval_secs`, 5–3600).

**Moving a probe** to another hub, company or site: enroll it again with a new code (a running probe follows within one interval). Renaming or moving it to another site of the same company is done in the hub's UI.

## 5. Run as a service

### Windows

The probe detects when the Service Control Manager starts it and runs as a service automatically. The package includes `install_probe_service.ps1`. Run it **as administrator** from the package folder:

```powershell
.\install_probe_service.ps1     # installs to C:\Blentinel\probe
& "C:\Blentinel\probe\probe.exe" enroll BLT1-…
```

It registers `BlentinelProbe` (automatic start, restart on failure). An upgrade replaces only the binary. Logs: `blentinel_probe.log` in that folder.

### Linux (systemd)

From the unpacked package folder:

```sh
sudo ./install_probe_service.sh
```

- It installs to `/opt/blentinel-probe`, the probe's own folder, which also holds its identity, enrollment, checks, buffer and log. It creates the system user `blentinel` if needed.
- It enables the `blentinel-probe` service. On a **first install** (no enrollment yet) it doesn't start it and prints the next steps:
  1. Get a code (Add probe) and run `sudo -u blentinel /opt/blentinel-probe/probe enroll <code>`.
  2. `sudo systemctl start blentinel-probe`.
  3. Add its checks on the hub.
- **Upgrading:** run the same script from the new package. It replaces **only the binary**. The identity, enrollment, checks and buffered reports are kept.
- **Ping without root:** the service grants the probe `CAP_NET_RAW`, which survives upgrades. Only when you run the binary by hand as a normal user do you need `sudo setcap cap_net_raw+ep /opt/blentinel-probe/probe`.

Logs: `journalctl -u blentinel-probe -f` and `/opt/blentinel-probe/blentinel_probe.log`.

## When the hub is unreachable

The probe keeps running its last checks (`config.json`) and buffers undelivered reports in `blentinel_blackbox.db` (up to 7 days / 20,000 reports), then delivers them oldest-first once the hub is back. Delivered reports are deleted immediately. Reports the hub rejects for good (for example from a probe it doesn't know) are dropped, not buffered. A probe the hub has **revoked** stops reporting, clears its buffer, and waits until it is enrolled again. Details in [architecture.md](../architecture.md#probe).

## Files in the probe folder

| File | What it is |
|---|---|
| `probe` / `probe.exe` | The binary |
| `config.json` | The checks and report interval the hub sent last, run after a restart and while the hub is unreachable. Written by the probe; changes are made on the hub. |
| `enrollment.json` | The hub (address, keys, certificate fingerprint) and the probe's company and site, written by `probe enroll`. Don't edit it; enroll again instead. Renamed `enrollment.revoked.json` if the hub revokes the probe. |
| `identity.key` | The probe's Ed25519 private key. **Its identity: back it up, never share it.** If it's lost, enroll the probe again (it gets a new ID). |
| `blentinel_blackbox.db` | Buffered reports (reset automatically if the probe or hub key changes) |
| `blentinel_probe.log` | Log file |

Problems? See [guide/troubleshooting.md](../guide/troubleshooting.md).
