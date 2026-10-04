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

`BLENTINEL_RELEASE_URL=<folder URL>` takes the release files from elsewhere (a mirror, or `file:///path` for an offline copy). Running it again upgrades in place, like `hub install`.

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

The hub offers probes for download, so **Add probe** can give a one-line install command per platform ([probe.md](probe.md)), and probes update themselves from it ([probe.md](probe.md#updates)). It keeps **signed releases** in `releases/<version>/` in its data folder, each with `manifest.json` (the SHA-256 of every binary), its signature, and the binaries, named `probe-<target>` (`.exe` for Windows) for these targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf` (Raspberry Pi), `x86_64-pc-windows-msvc`.

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

```sh
sudo hub backup                       # → /var/lib/blentinel/backups/blentinel-hub-<date>.tar.gz
sudo hub backup --out /mnt/nas/hub.tar.gz
```

A backup is one `.tar.gz` with a consistent copy of the database (made while the hub runs), the identity key, the TLS certificate and key, and the config. **It holds the hub's private key**: it is written readable by its owner only; keep it like the key itself. Copy backups off the server.

To restore, **stop the hub**, then:

```sh
sudo systemctl stop blentinel-hub         # Windows: Stop-Service BlentinelHub
sudo hub restore blentinel-hub-<date>.tar.gz
sudo systemctl start blentinel-hub
```

It puts the database, key and TLS files where the current config says (a new server keeps its own paths), moves the files it replaces to `<data>/before-restore-<date>/`, and saves the backed-up config as `blentinel_hub.toml.from-backup` beside the current one, for reference. It refuses a backup made by a newer hub. On a new server: `hub install`, then `hub restore`. Enrolled probes keep working because the key is the same.

## Upgrading

Run the new binary's `install` again: it stops the service, replaces the program, keeps the config and data, and starts it. New database migrations run at startup.

Blentinel is **pre-alpha** (DECISIONS D-30): an upgrade may change things without migrating old data. Check `CHANGELOG.md` for **Breaking** entries, and take a `hub backup` first. Upgrade the **hub before the probes**. Put the new probe builds beside the new hub (`--with-probes`): `install` adds them as a release, which Add probe offers. To update the running probes to it, approve it on the admin page ([probe.md](probe.md#updates)).

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
