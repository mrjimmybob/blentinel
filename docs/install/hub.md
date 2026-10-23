# Installing the hub

**Platforms:** Linux (x86_64, ARM64) with systemd is supported. Windows works but is best effort (DECISIONS D-36).

The hub is **one file** (the web UI is inside it, D-51) that installs, checks, backs up and removes itself.

## One-line install (Linux)

```sh
curl -fsSL https://github.com/mrjimmybob/blentinel/releases/latest/download/hub.sh | sudo sh
```

`hub.sh` installs the latest release from GitHub (D-57):

1. downloads `SHA256SUMS` and checks its **signature** against the release keys written into the script (with `openssl`);
2. downloads the hub for this machine (`x86_64` or `aarch64`, a static build that runs on any distribution with systemd) and the signed probe release, and checks each against its **signed SHA-256**;
3. runs `hub install` (below), which also adds the probes so **Add probe** can offer them.

If any check fails, nothing is installed. It needs `curl` (or `wget`), `openssl`, `sha256sum` and `tar`, as any Debian or Ubuntu has. Options for `hub install` go after `-s --`:

```sh
curl -fsSL https://github.com/mrjimmybob/blentinel/releases/latest/download/hub.sh | sudo sh -s -- --https-port 443
curl -fsSL https://github.com/mrjimmybob/blentinel/releases/latest/download/hub.sh | sudo BLENTINEL_VERSION=0.2.0 sh    # a given release instead of the latest
```

With `--https-port 443`, a hub reached by a public name can then get its certificate from Let's Encrypt on the admin page ([https.md](https.md#a-certificate-from-lets-encrypt)).

`BLENTINEL_RELEASE_URL=<folder URL>` takes the release files from elsewhere (a mirror, or `file:///path` for an offline copy). Running it again upgrades in place, like `hub install`; to upgrade, `hub upgrade` is safer (below).

**Not piping to a shell?** Download `hub.sh`, read it (about 100 lines), and check it against the release's `SHA256SUMS`; then `sudo sh hub.sh`. The script trusts only the release keys in it, so a mirror or a tampered download of the hub or probes is refused; the script itself comes over HTTPS from GitHub.

**Windows:** download `hub.exe` (build it, below) and run `.\hub.exe install` in an elevated PowerShell.

The rest of this page builds the hub from source, which is also how releases are made ([CONTRIBUTING.md](../../CONTRIBUTING.md#publishing)).

## 1. Build

```sh
cargo build -p blentinelmake --release
./target/release/blentinelmake release keygen      # once: the key that signs probe releases
./target/release/blentinelmake hub publish --with-probes x86_64-unknown-linux-musl,x86_64-pc-windows-msvc
```

`release keygen` creates the **release key** on this machine (once; [probe.md](probe.md#updates)). Keep its private half here and back it up: it signs every probe release, and never goes to the hub. Hubs and probes built afterwards trust it.

`publish/hub-<date>/app/` holds the `hub` binary (`hub.exe` on Windows), its checksum, and, with `--with-probes`, a `probes/` folder: a **signed release** of the probe for each platform listed. Copy the binary and `probes/` to the server, side by side.

## 2. Install

**Linux:**

```sh
sudo ./hub install
```

**Windows** (an elevated PowerShell):

```powershell
.\hub.exe install
```

It copies itself into place, creates the config on a first install, makes the data folder private, registers the service, starts it, and ends with where to go:

```
Blentinel hub 0.1.0 installed in /opt/blentinel/hub
  Config: /etc/blentinel/blentinel_hub.toml
  Data:   /var/lib/blentinel
  Probes: 2 binaries offered for download (Add probe)
  Service: blentinel-hub (starts with the machine, restarts on failure)

Open https://myserver:3443/
and create the first Owner with the setup code: K7PMX2QA
```

| | Linux | Windows |
|---|---|---|
| Program | `/opt/blentinel/hub/hub` | `C:\Program Files\Blentinel\Hub\hub.exe` |
| Config | `/etc/blentinel/blentinel_hub.toml` (root, readable by the service) | `C:\ProgramData\Blentinel\Hub\blentinel_hub.toml` |
| Data: database, keys, TLS files, setup code, archives, backups | `/var/lib/blentinel` (`0700`, the service user only) | `C:\ProgramData\Blentinel\Hub` (SYSTEM and Administrators only) |
| Service | systemd unit `blentinel-hub`, user `blentinel` (created) | Windows service `BlentinelHub` (LocalSystem) |
| Log | `journalctl -u blentinel-hub -f` | `C:\ProgramData\Blentinel\Hub\hub.log` |

A first install listens on **all interfaces**: port **3000** (plain HTTP, probes only) and **3443** (HTTPS: the web UI, and probes). Open both in the firewall. Other choices, on the first install only:

```sh
sudo ./hub install --host 10.0.0.5 --port 3000 --https-port 443
sudo ./hub install --no-service     # files only (e.g. a container runs the hub itself)
```

Every setting is in the config file, described in [reference/configuration.md](../reference/configuration.md); the hub applies changes to it within a second, without a restart.

Open the address it printed. The browser warns about the **self-signed certificate** the first time: compare its fingerprint with the one in the hub's log, then accept it ([https.md](https.md)). **Set up Blentinel** asks for the setup code and creates the first Owner. Then add users under **Users** ([guide/users.md](../guide/users.md)), add a company and site under **Companies**, and [install a probe](probe.md) with **Add probe**.

## Probe binaries

The hub offers probes for download, so **Add probe** can give a one-line install command per platform ([probe.md](probe.md)), and probes update themselves from it ([probe.md](probe.md#updates)). It keeps **signed releases** in `releases/<version>/` in its data folder, each with `manifest.json` (the SHA-256 of every binary), its signature, and the binaries, named `probe-<target>` (`.exe` for Windows) for these targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf` (Raspberry Pi 2 and later, 32-bit), `arm-unknown-linux-musleabihf` (Raspberry Pi Zero and Pi 1, ARMv6), `x86_64-pc-windows-msvc`.

A release is added by `hub install` (from the `probes/` folder beside the binary), on the admin page (**Probe updates > Add release**, the `.tar.gz` from `blentinelmake release build`), or with:

```sh
sudo hub release add publish/probe-release-0.2.0      # or the .tar.gz
hub release list
```

The hub takes only a release signed with a release key it was built to trust, whose binaries match the manifest, for a protocol it speaks. A hub built without a key takes none (the admin page says so). Add probe offers the release approved for updates, else the newest one. With none, Add probe shows the code and `probe install --code …` to run by hand.

Downloads (`/download/probe/<target>`) need no login and are served on both ports, plain HTTP included: a machine being set up has no session, and Windows PowerShell 5.1 can't follow a redirect to HTTPS. That's safe because the command from Add probe checks the file's SHA-256, which it got from the hub's HTTPS page, before running it ([security.md](../security.md#probe-downloads)).

## Checking on it

```sh
hub status      # service, ports, database, probes, certificate expiry, pending setup code
hub doctor      # checks, with what to do about each problem
```

`hub doctor` checks the config, the service, the data folder (exists, writable, private), the identity key, the database (opens, consistent, not from a newer hub), the TLS certificate (expired, or expiring within 30 days), the ports (answering, or free), and disk space. It exits with `1` if anything fails. Run both as root/Administrator (the data folder is private).

Without `--config` these commands use `blentinel_hub.toml` in the current folder if there is one, else the installed hub's config.

## Backups

**The hub backs itself up every day** (at 03:00 UTC, keeping the last 7) without anyone setting it up. Change that on the **admin page, Backups**: on or off, the hour (UTC), how many to keep (1 to 365). The section lists the backups (when, why, size), shows when the next one runs and whether the last one failed, and has **Back up now** and **Delete**. They are written to the backups folder, `<data>/backups/` unless `[backups] path` in the config says otherwise ([configuration](../reference/configuration.md#backups)).

From the server's command line:

```sh
sudo hub backup                       # → /var/lib/blentinel/backups/blentinel-hub-<date>.tar.gz
sudo hub backup --out /mnt/nas/hub.tar.gz
```

A backup is one `.tar.gz` with a consistent copy of the database (made while the hub runs), the identity key, the data key (`hub_data.key`, which seals the alert channels' secrets in the database), the TLS certificate and key, and the config. **It holds the hub's private key**: it is written readable by its owner only; keep it like the key itself. Backups on the server's own disk don't survive losing that disk: point `[backups] path` at another disk or a mounted share, or copy the files off the server. **Download** on the admin page asks for your password again, because of the key in them, then saves the file.

Which backups are kept, by the end of the file name:

| File | Made by | Kept |
|---|---|---|
| `blentinel-hub-<date>.tar.gz` | **Back up now**, `hub backup` | until someone deletes it |
| `blentinel-hub-<date>-scheduled.tar.gz` | the daily backup | the newest *N* (the admin page's "keep") |
| `blentinel-hub-<date>-pre-upgrade.tar.gz` | a new hub, before it upgrades the database | the newest 3 |

A daily backup that fails is shown on the admin page and in the log; the next try is the next day's hour (or **Back up now**). A hub that was off at the hour backs up when it starts again.

To restore, **stop the hub**, then:

```sh
sudo systemctl stop blentinel-hub         # Windows: Stop-Service BlentinelHub
sudo hub restore blentinel-hub-<date>.tar.gz
sudo systemctl start blentinel-hub
```

It puts the database, key and TLS files where the current config says (a new server keeps its own paths), moves the files it replaces to `<data>/before-restore-<date>/`, and saves the backed-up config as `blentinel_hub.toml.from-backup` beside the current one, for reference. It refuses a backup made by a newer hub. On a new server: `hub install`, then `hub restore`. Enrolled probes keep working because the key is the same.

## Rotating the certificate and key

Probes pin the hub's key and its certificate's public key (D-47, D-61). To replace either, use the admin page, **Hub keys**, instead of changing the files: probes are told before the hub switches.

1. **Start rotation:** choose the certificate, the hub key, or both, and a window (14 days by default, 1 to 90). The hub makes the new ones (a new self-signed certificate with a new key pair; a new X25519 key) beside the current ones: `hub_identity.next.key`, `hub_tls_cert.next.pem`, `hub_tls_key.next.pem`. With [Let's Encrypt](https.md#a-certificate-from-lets-encrypt) on, the hub then gets the next certificate from it, for the new key, so the switch serves a trusted one.
2. **Probes take them** with their next report and keep the old ones too. The section shows how many hold them, and which don't and why (a probe older than this version must update itself first: approve its release under **Probe updates**).
3. **The hub switches by itself** once every active probe holds them: it serves the new certificate without a restart and signs with the new key, and probes drop the old pins. **Switch now** switches earlier: probes that aren't ready then can't reach the hub until they are enrolled again (Add probe's command). After the window, a rotation still waiting is flagged on the page and in the log, but not forced.
4. **Cancel rotation** drops the new files; nothing changes.

At the switch, enrollment codes made before stop working (they carry the old pins): make new ones. After a key switch the previous key stays as `hub_identity.prev.key` for 7 days, so reports probes buffered before they switched still open; then it is deleted. Backups include these files.

Rotation is for routine replacement. If the key or the TLS key was **stolen**, rotation doesn't help (the thief can sign a rotation too): give the hub a new key and certificate by hand and enroll every probe again ([security.md](../security.md#enrollment)).

## Upgrading

**The hub never upgrades itself** (D-41, D-75): an admin runs the upgrade on the server.

```sh
sudo /opt/blentinel/hub/hub upgrade                     # to the latest release ([upgrades] release_url: GitHub)
sudo /opt/blentinel/hub/hub upgrade --from /srv/blentinel-0.5.0   # from a folder of release files (offline, a mirror)
```

**Windows:** hub releases (`blentinelmake release hub`) carry Linux hubs only, so `hub upgrade` says there is no hub for this machine. Upgrade a Windows hub by running the new `hub.exe install` in an elevated terminal (below); the database is still backed up first, and put back if its upgrade fails (D-60).

`hub upgrade`:

1. checks the release's `SHA256SUMS` against the release keys built into this hub, and the new hub's SHA-256 against it (a mirror or a tampered file is refused). It takes only a **newer** version;
2. runs the new hub's `self-test` beside the running one: it reads the config and key and upgrades a **copy** of the database. If that fails, nothing has changed;
3. backs the hub up (a `-pre-upgrade` backup in the backups folder);
4. stops the service, keeps the running program as `hub.prev`, puts the new one in place, and starts it;
5. waits for the new hub to run and answer (90 s to start, then 15 s steady). If it doesn't, it **puts back the old program and the database** from that backup, starts the old hub again, and says where the new hub's database went (`failed-upgrade-<date>/`);
6. adds the release's probes: approve them under **Probe updates** to update the running probes.

**Knowing when there's a new version:** on the admin page, **Hub Version** has **Check daily** (off by default, since it means contacting GitHub or your `[upgrades] release_url`) and **Check now**. When a newer signed release is out, the section says so and shows the command above. It only tells you; it never upgrades.

Running the new binary's `install` again also upgrades (it stops the service, replaces the program, keeps the config and data, and starts it), without the self-test or the way back.

**A staging hub** (the lab's, D-106) also accepts releases signed with a **staging key**: internal development builds, which are never published as releases.
- **Making one:** `hub install --staging` on a first install, `BLENTINEL_STAGING=1` with `hub.sh`, or `[upgrades] staging = true` in the config ([configuration](../reference/configuration.md#upgrades)).
- **Every other hub refuses them.** The builds carry the staging keys, but only a hub that opted in on its own server trusts them, so a production hub never runs one.

**When a new hub has to upgrade the database, it backs the hub up first** (a `-pre-upgrade` backup in the backups folder, D-60), then upgrades it. If the upgrade fails, the hub puts the database back as it was, keeps the one that failed in `<data>/failed-upgrade-<date>/`, and doesn't start: its log says so. Install the previous version again to keep going, and report the error. If the backup itself can't be made, nothing is changed and the hub doesn't start either.

Blentinel is **pre-alpha** (DECISIONS D-30): an upgrade may change things without migrating old data. Check `CHANGELOG.md` for **Breaking** entries. Upgrade the **hub before the probes**. Put the new probe builds beside the new hub (`--with-probes`): `install` adds them as a release, which Add probe offers. To update the running probes to it, approve it on the admin page ([probe.md](probe.md#updates)).

## Uninstalling

```sh
sudo hub uninstall                  # removes the service and the program; keeps config and data
sudo hub uninstall --delete-data    # also deletes the config and the data folder (backups included)
```

On Windows the running `hub.exe` can't delete its own folder: delete `C:\Program Files\Blentinel\Hub` afterwards.

## Running it by hand

For a trial, or in a container, the hub runs without being installed:

```sh
./hub --init          # writes a commented blentinel_hub.toml in the current folder
./hub                 # or: ./hub --config /path/to/blentinel_hub.toml
```

It creates its data (database, keys, TLS files, setup code) in `state_dir` from the config (`.` by default). The web UI is on `https://<server>:3443`; the template listens on `127.0.0.1` only, so set `host = "0.0.0.0"` for other machines.
