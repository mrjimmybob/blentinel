# Installing the hub

> **Pre-alpha.** There are no pre-built binaries yet: build from source ([CONTRIBUTING.md](../../CONTRIBUTING.md)). A one-line download-and-install script is planned (ROADMAP M2-10).

**Platforms:** Linux (x86_64, ARM64) with systemd is supported. Windows works but is best effort (DECISIONS D-36).

The hub is **one file** (the web UI is inside it, D-51) that installs, checks, backs up and removes itself.

## 1. Build

```sh
cargo build -p blentinelmake --release
./target/release/blentinelmake hub publish
```

`publish/hub-<date>/app/` holds the `hub` binary (`hub.exe` on Windows) and its checksum. Copy the binary to the server.

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

Blentinel is **pre-alpha** (DECISIONS D-30): an upgrade may change things without migrating old data. Check `CHANGELOG.md` for **Breaking** entries, and take a `hub backup` first. Upgrade the **hub before the probes**.

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
