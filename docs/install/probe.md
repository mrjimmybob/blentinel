# Installing a probe

A probe is one self-contained binary that runs inside a client network and pushes reports to the hub. It needs **outbound** access to the hub's address and nothing inbound. Besides its reports, it keeps one request open to the hub (renewed every 25 seconds) to learn of commands at once. Every hub uses the same probe binary: which hub it trusts comes from its **enrollment code**, not from the build. There is no config file: what it checks is set on the hub.

**Platforms:** Linux with systemd (any distribution: the binary is static) and Windows.

## 1. Add probe (on the hub)

In the hub's web UI: **Companies** → the company → **Add probe**. If the company doesn't exist yet, add it first (Admins add companies; see [guide/companies-sites.md](../guide/companies-sites.md)). Add probe stays disabled until the company has a site: add one under **Sites**.

- **Site:** where the probe is.
- **Probe name:** optional; empty means the machine's hostname.
- **Limit:** how many probes may use the code. Empty (the default) means no limit: one code sets up as many machines as you need until it expires. Revoke it under **Codes still valid** when you're done, or set a limit (1 for exactly one machine).
- **Valid for:** how long the code works (24 hours by default).
- **Hub address, as probes reach it:** pre-filled with the address you're using in the browser (`https://<hub>:3443`). Change it if the probe reaches the hub by another name or address.

The hub shows the enrollment code **once** (`BLT1-…`: it carries the hub's address, its keys and a pin of its certificate's key, so the probe trusts that hub from the first packet), and **one line to run** for each platform it has a probe binary for.

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

If the hub offers no binary for a platform, copy the probe there yourself (build: [CONTRIBUTING.md](../../CONTRIBUTING.md)) and run `probe install --code BLT1-…` as root / Administrator. The hub offers the binaries of its signed releases ([hub install](hub.md#probe-binaries)).

| | Linux | Windows |
|---|---|---|
| Folder (program, identity, enrollment, checks, buffer, log) | `/var/lib/blentinel-probe` (`0700`) | `C:\Program Files\Blentinel\Probe` (SYSTEM and Administrators only) |
| Service | systemd `blentinel-probe`, run as its own user `blentinel-probe` (no login, no password), allowed to ping (`CAP_NET_RAW`), writing nowhere but its folder | Windows service `BlentinelProbe` (LocalSystem, restarts on failure) |
| Outside the folder | the unit `/etc/systemd/system/blentinel-probe.service`; the user and group `blentinel-probe` | the service's registration |
| Log | `journalctl -u blentinel-probe` and `blentinel_probe.log` in the folder | `blentinel_probe.log` in the folder |

On Linux the install talks to systemd over D-Bus, and makes the service's user with the system's `useradd` (D-84). Nothing is installed on the machine: the probe uses only what every systemd machine has. `probe uninstall` removes the unit and the user again.

## 3. Give it checks (on the hub)

**What it checks, and how often, is set on the hub**: the hub sends the probe its checks and report interval in its signed answer to a report. The probe runs a new set at once and keeps it in `config.json`, so after a restart, or while the hub is unreachable, it runs the last checks it received. A new probe checks nothing until it has checks, and reports every 60 seconds.

Open the probe's page (Companies → the company → click the probe's name):

- **Add check:** a name and what to check (ping, web page, TCP port, TLS certificate, DNS name, database server, mail server, directory server (LDAP, Active Directory), time server (NTP), UPS (NUT), SNMP values, CPU, memory, one disk or every disk, the drives' health (SMART), the machine's temperature, its Hyper-V virtual machines, a file or folder, a service, a process, a network interface, a Windows event log, pending updates and restart, uptime), with that kind's settings (a web page: the status codes that are fine, text it must or mustn't contain, headers, whether to follow redirects). Values are checked before they're saved.
- **Network devices:** **Templates…** has a printer, a switch's ports, a Synology or QNAP NAS and an APC UPS over SNMP: type the device's address and its SNMP community ([checks](../reference/checks.md#templates)).
- **Suggested checks:** once the probe has reported, the hub proposes checks from what its machine is made of (CPU, memory, its disks, its gateway, the services it runs that people usually watch). **Add all suggested** is the quickest start; the **Machine** section shows what the probe found.
- **Templates…:** a set of checks in one go (a machine baseline, a website, a backup folder, a domain controller), or your own: save one probe's checks as a template and apply it to the next.
- **Copy checks from…** another probe: for a new machine like one you already monitor.
- **Edit**, **Disable** and **Delete** each check; set **Report every (seconds)** (5–3600).
- The badge says whether the probe runs the current checks: **In sync**, **Pending** (it gets them with its next report) or **Rejected** with the probe's reason.
- **Run checks now:** the probe gets the request within a second or two, checks at once and reports straight away, instead of an interval later. Between reports it keeps one request open to the hub asking for commands (D-76), which the hub answers as soon as there is one; if that request can't stay open (a proxy that cuts quiet connections), it asks every 30 seconds, and the command also comes with its next report. **Commands** lists what was asked, by whom, and what came of it (`ran 3 checks`); one not sent yet can be cancelled, and one the probe doesn't pick up within 3 intervals (at least 15 minutes) expires. The hub never connects to the probe: it always answers a request the probe made.

The kinds and their settings are in [reference/checks.md](../reference/checks.md).

## Checking on it

```sh
sudo /var/lib/blentinel-probe/probe status     # Windows: & "C:\Program Files\Blentinel\Probe\probe.exe" status
sudo /var/lib/blentinel-probe/probe doctor
```

`status` shows the service, the probe's ID, its hub, company and site, its checks and interval, and reports waiting to be sent. `doctor` checks the service, the folder (private), the identity, the enrollment, **whether the hub answers through its pinned certificate**, the clock against the hub's (the hub refuses reports more than 5 minutes off), the checks and the buffer, with what to do about each problem; it sends no report and exits `1` if anything fails. Run them as root / Administrator: the folder is private.

**Without logging in to the machine:** each report tells the hub how the probe itself is doing (D-73): since when it runs, the reports it holds back, its clock against the hub's, and its last problem of the past day. The hub's **Probes** page lists every probe with these, its version, its last report and whether it runs its checks, and flags those that need attention; the probe's own page has a **Health** section.

## Updates

**Probes update themselves from their hub** (D-55), so nobody needs to reach the machines. Nothing changes until an admin chooses the version, on the hub's admin page under **Probe updates**:

1. Add the release (built with `blentinelmake release build --targets …`, or shipped with the hub) and click **Approve**. **Stop updates** withdraws it.
2. With its answer to each report, the hub offers that release to the probes that should get it now (see **Rollout** below).
3. The probe installs it only if the release is **signed with a release key built into the probe** (the hub can't sign one), it is **newer** than the probe (or a signed rollback allows going back, below), and it has a binary for this machine. It downloads the binary from the hub while monitoring goes on, checks its SHA-256 against the signed manifest, and runs it with `--self-test` (it must start, read the probe's files and reach the hub).
4. Then it keeps the running program as `probe.prev`, puts the new one in place and restarts (systemd and the Windows service start it again; a probe run by hand starts it itself).
5. The new version must deliver a report within 15 minutes (or 3 intervals) and may start at most 3 times before it does. Otherwise the probe puts `probe.prev` back, and doesn't try that version again.

The admin page lists every probe's version and its **Update** status: up to date, updating to a version, or why it waits (after the canaries, paused, outside its window, off for its company…). A refused or rolled-back update shows there, and as **update failed** in the company's probe list, with the probe's reason. The hub doesn't offer that version to that probe again: approve a newer one.

### Rollout: canaries first

Tick **Canary** for a few probes in the probe list (a lab machine, a friendly client). An approved release then goes to the canaries first (D-74):

- Once **every canary runs it**, the hub waits the soak time you set when approving (60 minutes by default; up to a week), then offers it to every probe on its own.
- If a canary **refuses it or rolls back**, the rollout **pauses** and the admin page says which canary and why. Nothing more is offered until someone acts.
- **Pause** stops offering it to more probes (probes already updating finish); **Resume** goes on. **Roll out to all now** skips the wait, and also resumes a paused rollout.
- With no canaries, an approved release goes to every probe at once. Canaries of a company that doesn't follow the rollout (below) don't count, nor do revoked ones.

### By company: off, pinned, a window

Under **Updates by company**, each company:

- **follows the rollout** (the default), or is **off** (its probes stay on what they run), or is **pinned** to one release whatever is rolled out (the pinned release goes to its probes directly, without canaries; it can't be removed from the hub while pinned);
- optionally has an **update window**, e.g. 02:00 to 05:00 (past midnight if the end is earlier), in **local time at each probe's site** (the site's time zone, set on the company page). Outside it, its probes wait; the hub decides, so nothing is set on the machines.

### Going back to an older release

Probes never install an older version on their own: a hacked hub could otherwise push an old, vulnerable one. To go back, sign a rollback on the build machine and add it to the hub:

```sh
blentinelmake release rollback --to 0.4.1 --from 0.4.2
```

It writes `publish/probe-rollback-0.4.1.tar.gz`: add it under **Probe updates** like a release (or `hub release add publish/probe-rollback-0.4.1`), then approve 0.4.1. Only probes running a `--from` version go back; the probe checks the rollback's signature against its built-in release keys, as it does the release's. Release 0.4.1 must still be on the hub (or added again).

**The local brake:** `probe updates off` (root / Administrator) makes this machine refuse every update, and the hub shows why; `probe updates on` undoes it. `probe status` shows whether updates are on, a version on trial, and the last failure.

Probes installed before M2-15 can't update themselves: install the new probe on them once (Add probe's command or `probe install`).

## Upgrading, moving, removing

- **Upgrade:** probes update themselves (above). To upgrade by hand, run `probe install` from the new binary. It stops the service, replaces the program, and starts it again; the identity, enrollment, checks and buffered reports are kept.
- **Move** to another hub, company or site: `probe install --code <new code>` enrolls it again in place. Renaming it, or moving it to another site of the same company, is done on the hub.
- **Remove:** `probe uninstall` removes the service, its user (Linux) and the program, and keeps the folder, now root's (installing again keeps the same probe ID); `probe uninstall --delete-data` removes everything: nothing is left behind. If the user can't be removed, the command says so and goes on: remove it with `sudo userdel blentinel-probe`. On Windows the running `probe.exe` can't delete itself: run `uninstall` from another copy, or delete the folder afterwards. Revoke the probe on the hub too.

`probe enroll <code>` still enrolls a probe in its own folder without installing anything, for running it by hand (`./probe`).

## When the hub is unreachable

The probe keeps running its last checks (`config.json`) and buffers undelivered reports in `blentinel_blackbox.db` (up to 7 days / 20,000 reports), then delivers them oldest-first once the hub is back. Delivered reports are deleted immediately. Reports the hub rejects for good (for example from a probe it doesn't know) are dropped, not buffered. A probe the hub has **revoked** stops reporting, clears its buffer, and waits until it is enrolled again. Details in [architecture.md](../architecture.md#probe).

## Files in the probe folder

| File | What it is |
|---|---|
| `probe` / `probe.exe` | The binary |
| `config.json` | The checks and report interval the hub sent last, run after a restart and while the hub is unreachable. Written by the probe; changes are made on the hub. Holds the checks' secrets (an SNMP community, v3 passwords): readable only by its owner. |
| `enrollment.json` | The hub (address, keys, certificate pin, and its next ones while it rotates them, D-61) and the probe's company and site, written at enrollment. Don't edit it; enroll again instead. Renamed `enrollment.revoked.json` if the hub revokes the probe. |
| `identity.key` | The probe's Ed25519 private key. **Its identity: back it up, never share it.** If it's lost, enroll the probe again (it gets a new ID). |
| `blentinel_blackbox.db` | Buffered reports (reset automatically if the probe or hub key changes) |
| `blentinel_probe.log` | Log file |
| `probe.prev`, `update.json`, `update-failed.json` | After an update: the previous version, the update on trial (until the new version reports), the last version it didn't install and why |
| `updates-off` | The local brake (`probe updates off`) |

Problems? Start with `probe doctor`, then [guide/troubleshooting.md](../guide/troubleshooting.md).
