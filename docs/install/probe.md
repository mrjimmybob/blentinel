# Installing a probe

A probe is one self-contained binary that runs inside a client network and pushes reports to the hub. It needs **outbound** access to the hub's URL and nothing inbound. Every hub uses the same probe binary: which hub it trusts comes from its **enrollment code**, not from the build.

> **Pre-alpha.** Build from source ([CONTRIBUTING.md](../../CONTRIBUTING.md)). One-command installation with per-OS one-liners is planned (ROADMAP M2-09).

## 1. Build for the target machine

```sh
./target/release/blentinelmake probe publish --target x86_64-unknown-linux-musl   # Linux x86_64
./target/release/blentinelmake probe publish --target aarch64-unknown-linux-musl  # Raspberry Pi 4/5
./target/release/blentinelmake probe publish --target x86_64-pc-windows-msvc      # Windows
```

Use the **musl** targets on Linux: the binary is fully static and runs on any distribution. Copy the resulting `app/` folder to the probe machine. Everything the probe creates (identity, enrollment, buffer, log) stays in its own folder.

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

## 4. Choose what to check, and start

```sh
./probe --init          # writes a commented blentinel_probe.toml next to the binary
```

```toml
[agent]
interval = 60           # seconds between reports

[[resources]]
name = "Core switch"
type = "ping"
target = "192.0.2.1"
```

A complete example is in [`probe/blentinel_probe.example.toml`](../../probe/blentinel_probe.example.toml), and every option is in [reference/configuration.md](../reference/configuration.md). (Managing checks from the hub, instead of this file, is ROADMAP M2-04.)

```sh
./probe
```

It reports every `interval` seconds; the probe appears on its company's page and on the dashboard. A probe started before it's enrolled waits and says so, then starts by itself once `enrollment.json` appears.

**Moving a probe** to another hub, company or site: enroll it again with a new code (a running probe follows within one interval). Renaming or moving it to another site of the same company is done in the hub's UI.

## 5. Run as a service

### Windows

The probe detects when the Service Control Manager starts it and runs as a service automatically. The package includes `install_probe_service.ps1`. Run it **as administrator** from the package folder:

```powershell
.\install_probe_service.ps1     # installs to C:\Blentinel\probe, keeps an existing config
& "C:\Blentinel\probe\probe.exe" enroll BLT1-…
```

It registers `BlentinelProbe` (automatic start, restart on failure) and keeps an existing `blentinel_probe.toml`. Logs: `blentinel_probe.log` in that folder.

### Linux (systemd)

From the unpacked package folder:

```sh
sudo ./install_probe_service.sh
```

- It installs to `/opt/blentinel-probe`, the probe's own folder, which also holds its identity, enrollment, config, buffer and log. It creates the system user `blentinel` if needed.
- It enables the `blentinel-probe` service. On a **first install** it doesn't start it yet and prints the next steps:
  1. Get a code (Add probe) and run `sudo -u blentinel /opt/blentinel-probe/probe enroll <code>`.
  2. Edit `/opt/blentinel-probe/blentinel_probe.toml` (the resources to check).
  3. `sudo systemctl start blentinel-probe`.
- **Upgrading:** run the same script from the new package. It replaces **only the binary**. The identity, enrollment, config and buffered reports are kept.
- **Ping without root:** the service grants the probe `CAP_NET_RAW`, which survives upgrades. Only when you run the binary by hand as a normal user do you need `sudo setcap cap_net_raw+ep /opt/blentinel-probe/probe`.

Logs: `journalctl -u blentinel-probe -f` and `/opt/blentinel-probe/blentinel_probe.log`.

## When the hub is unreachable

The probe keeps checking and buffers undelivered reports in `blentinel_blackbox.db` (up to 7 days / 20,000 reports), then delivers them oldest-first once the hub is back. Delivered reports are deleted immediately. Reports the hub rejects for good (for example from a probe it doesn't know) are dropped, not buffered. A probe the hub has **revoked** stops reporting, clears its buffer, and waits until it is enrolled again. Details in [architecture.md](../architecture.md#probe).

## Files in the probe folder

| File | What it is |
|---|---|
| `probe` / `probe.exe` | The binary |
| `blentinel_probe.toml` | Interval and resources (hot-reloaded on change) |
| `enrollment.json` | The hub (address, keys, certificate fingerprint) and the probe's company and site, written by `probe enroll`. Don't edit it; enroll again instead. Renamed `enrollment.revoked.json` if the hub revokes the probe. |
| `identity.key` | The probe's Ed25519 private key. **Its identity: back it up, never share it.** If it's lost, enroll the probe again (it gets a new ID). |
| `blentinel_blackbox.db` | Buffered reports (reset automatically if the probe or hub key changes) |
| `blentinel_probe.log` | Log file |

Problems? See [guide/troubleshooting.md](../guide/troubleshooting.md).
