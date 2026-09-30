# Installing a probe

A probe is one self-contained binary that runs inside a client network and pushes reports to the hub. It needs **outbound** access to the hub's address and nothing inbound. Every hub uses the same probe binary: which hub it trusts comes from its **enrollment code**, not from the build. There is no config file: what it checks is set on the hub.

**Platforms:** Linux with systemd (any distribution: the binary is static) and Windows.

## 1. Add probe (on the hub)

In the hub's web UI: **Companies** → the company → **Add probe**. If the company doesn't exist yet, add it first (Admins add companies; see [guide/companies-sites.md](../guide/companies-sites.md)). Add probe stays disabled until the company has a site: add one under **Sites**.

- **Site:** where the probe is.
- **Probe name:** optional; empty means the machine's hostname.
- **Limit:** how many probes may use the code. Empty (the default) means no limit: one code sets up as many machines as you need until it expires. Revoke it under **Codes still valid** when you're done, or set a limit (1 for exactly one machine).
- **Valid for:** how long the code works (24 hours by default).
- **Hub address, as probes reach it:** pre-filled with the address you're using in the browser (`https://<hub>:3443`). Change it if the probe reaches the hub by another name or address.

The hub shows the enrollment code **once** (`BLT1-…`: it carries the hub's address, its keys and its certificate's fingerprint, so the probe trusts that hub from the first packet), and **one line to run** for each platform it has a probe binary for.

## 2. Run the line on the machine

**Linux**, in a shell (it asks for `sudo`):

```sh
curl -fsSLk -o probe 'https://hub.example.com:3443/download/probe/x86_64-unknown-linux-musl' && echo '6b2598…  probe' | sha256sum -c - && chmod +x probe && sudo ./probe install --code BLT1-…
```

**Windows**, in PowerShell **as Administrator** (Windows PowerShell 5.1 or PowerShell 7):

```powershell
$ErrorActionPreference='Stop'; $f="$env:TEMP\blentinel-probe.exe"; … Invoke-WebRequest '…/download/probe/x86_64-pc-windows-msvc' -OutFile $f @o; $h=…; if ($h -eq '6B2598…') { & $f install --code BLT1-… } else { throw 'checksum mismatch' }
```

Copy the line from the hub: it holds that code and that file's checksum. It:

1. **downloads** the probe from the hub (any certificate is accepted for the download);
2. **checks its SHA-256** against the one shown on the hub's HTTPS page. That check is what proves the file is the hub's; the line runs nothing if it fails, or if any step fails;
3. runs **`probe install --code …`**, which copies the probe into its folder, **enrolls** it (the probe then pins the hub's certificate and keys from the code), registers the **service** and starts it.

```
Blentinel probe 0.1.0 installed in /var/lib/blentinel-probe
  Service: blentinel-probe (starts with the machine, restarts on failure)
  Reports to https://hub.example.com:3443 as "FRONTDESK-PC" (ACME Ltd, Headquarters)

Give it its checks on the hub: open the probe's page (its name on the company's page).
```

If the hub offers no binary for a platform, copy the probe there yourself (build: [CONTRIBUTING.md](../../CONTRIBUTING.md)) and run `probe install --code BLT1-…` as root / Administrator. The hub offers what's in its `probes/` folder ([hub install](hub.md#probe-binaries)).

| | Linux | Windows |
|---|---|---|
| Folder (program, identity, enrollment, checks, buffer, log) | `/var/lib/blentinel-probe` (`0700`) | `C:\Program Files\Blentinel\Probe` (SYSTEM and Administrators only) |
| Service | systemd `blentinel-probe`, run as a throwaway user (`DynamicUser`), allowed to ping (`CAP_NET_RAW`), nothing else | Windows service `BlentinelProbe` (LocalSystem, restarts on failure) |
| Log | `journalctl -u blentinel-probe` and `blentinel_probe.log` in the folder | `blentinel_probe.log` in the folder |

On Linux the install talks to systemd over D-Bus (`systemctl` isn't used: the probe runs no external tools, FP-2); systemd keeps the folder in `/var/lib/private/blentinel-probe` behind the `/var/lib/blentinel-probe` link.

## 3. Give it checks (on the hub)

**What it checks, and how often, is set on the hub**: the hub sends the probe its checks and report interval in its signed answer to a report. The probe runs a new set at once and keeps it in `config.json`, so after a restart, or while the hub is unreachable, it runs the last checks it received. A new probe checks nothing until it has checks, and reports every 60 seconds.

Open the probe's page (Companies → the company → click the probe's name):

- **Add check:** a name and what to check (ping, web page, TCP port, CPU, memory, disk, uptime), with that kind's settings. Values are checked before they're saved.
- **Copy checks from…** another probe: for a new machine like one you already monitor.
- **Edit**, **Disable** and **Delete** each check; set **Report every (seconds)** (5–3600).
- The badge says whether the probe runs the current checks: **In sync**, **Pending** (it gets them with its next report) or **Rejected** with the probe's reason.

The kinds and their settings are in [reference/checks.md](../reference/checks.md).

## Checking on it

```sh
sudo /var/lib/blentinel-probe/probe status     # Windows: & "C:\Program Files\Blentinel\Probe\probe.exe" status
sudo /var/lib/blentinel-probe/probe doctor
```

`status` shows the service, the probe's ID, its hub, company and site, its checks and interval, and reports waiting to be sent. `doctor` checks the service, the folder (private), the identity, the enrollment, **whether the hub answers through its pinned certificate**, the clock against the hub's (the hub refuses reports more than 5 minutes off), the checks and the buffer, with what to do about each problem; it sends no report and exits `1` if anything fails. Run them as root / Administrator: the folder is private.

## Upgrading, moving, removing

- **Upgrade:** run `probe install` from the new binary. It stops the service, replaces the program, and starts it again; the identity, enrollment, checks and buffered reports are kept.
- **Move** to another hub, company or site: `probe install --code <new code>` enrolls it again in place. Renaming it, or moving it to another site of the same company, is done on the hub.
- **Remove:** `probe uninstall` removes the service and the program and keeps the folder (installing again keeps the same probe ID); `probe uninstall --delete-data` removes everything: nothing is left behind. On Windows the running `probe.exe` can't delete itself: run `uninstall` from another copy, or delete the folder afterwards. Revoke the probe on the hub too.

`probe enroll <code>` still enrolls a probe in its own folder without installing anything, for running it by hand (`./probe`).

## When the hub is unreachable

The probe keeps running its last checks (`config.json`) and buffers undelivered reports in `blentinel_blackbox.db` (up to 7 days / 20,000 reports), then delivers them oldest-first once the hub is back. Delivered reports are deleted immediately. Reports the hub rejects for good (for example from a probe it doesn't know) are dropped, not buffered. A probe the hub has **revoked** stops reporting, clears its buffer, and waits until it is enrolled again. Details in [architecture.md](../architecture.md#probe).

## Files in the probe folder

| File | What it is |
|---|---|
| `probe` / `probe.exe` | The binary |
| `config.json` | The checks and report interval the hub sent last, run after a restart and while the hub is unreachable. Written by the probe; changes are made on the hub. |
| `enrollment.json` | The hub (address, keys, certificate fingerprint) and the probe's company and site, written at enrollment. Don't edit it; enroll again instead. Renamed `enrollment.revoked.json` if the hub revokes the probe. |
| `identity.key` | The probe's Ed25519 private key. **Its identity: back it up, never share it.** If it's lost, enroll the probe again (it gets a new ID). |
| `blentinel_blackbox.db` | Buffered reports (reset automatically if the probe or hub key changes) |
| `blentinel_probe.log` | Log file |

Problems? Start with `probe doctor`, then [guide/troubleshooting.md](../guide/troubleshooting.md).
