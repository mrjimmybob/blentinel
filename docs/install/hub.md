# Installing the hub

> **Pre-alpha.** There are no pre-built binaries yet: build from source ([CONTRIBUTING.md](../../CONTRIBUTING.md)). One-command installation is planned (ROADMAP M2, proposal 0004).

## 1. Build and deploy

```sh
cargo build -p blentinelmake --release
./target/release/blentinelmake hub publish
```

Copy `publish/hub/app/` to the server (for example `/opt/blentinel/hub`). It contains the `hub` binary, its web assets (`pkg/`) and a sample config. The hub detects the `pkg/` folder next to it and serves the UI from there.

## 2. Configure

```sh
./hub --init            # writes a commented blentinel_hub.toml
```

Edit `blentinel_hub.toml`. At minimum, set:

```toml
[server]
host = "0.0.0.0"                  # listen on all interfaces
port = 3000
state_dir = "/var/lib/blentinel"  # database, keys and token live here
db_path = "blentinel.db"          # relative to state_dir
```

Every setting is described in [reference/configuration.md](../reference/configuration.md). To use another config location, start with `./hub --config /path/to/blentinel_hub.toml`.

## 3. First start

```sh
./hub
```

On first start the hub creates, in `state_dir`:

| File | What it is |
|---|---|
| `hub_identity.key` | The hub's X25519 private key (probes seal reports for it). **Protect it.** |
| `hub_identity.pub` | Its public key, in hex (for `hub_public_key` in probe configs) |
| `hub_auth.token` | The admin token for the web UI. **Printed once** to the terminal. |
| `blentinel.db` | The database. Its schema is created automatically. |

Open `http://<server>:3000` and log in with the admin token. Lost it? `./hub --reset-admin-token` generates a new one.

Next: [install a probe](probe.md) and register it.

## 4. Run as a service

The package includes an installer that also handles **upgrades**: it replaces only the program files, never the configuration or the data.

### Linux (systemd)

From the unpacked package folder:

```sh
sudo ./install_hub_service.sh
```

| What | Where |
|---|---|
| Program (binary + web assets) | `/opt/blentinel/hub` (replaced on upgrade) |
| Configuration | `/etc/blentinel/blentinel_hub.toml` (written on first install only) |
| Data: database, keys, admin token, archives, TLS files | `/var/lib/blentinel` (never touched by the installer) |
| Service | `blentinel-hub`, runs as the system user `blentinel` (created if missing) |

After the first install, set `host = "0.0.0.0"` in the config so probes on other machines can reach the hub, then `sudo systemctl restart blentinel-hub`. The admin token is in `/var/lib/blentinel/hub_auth.token`. Logs: `journalctl -u blentinel-hub -f`.

### Windows

From the unpacked package folder, in an **elevated** PowerShell:

```powershell
.\install_hub_service.ps1
```

The hub doesn't have a Windows-service mode yet, so the installer registers a **scheduled task** ("Blentinel Hub") that starts at boot as SYSTEM and restarts the hub if it stops.

| What | Where |
|---|---|
| Program | `C:\Program Files\Blentinel\Hub` (replaced on upgrade) |
| Configuration and data | `C:\ProgramData\Blentinel\Hub` (never overwritten) |

After the first install, set `host = "0.0.0.0"` in `C:\ProgramData\Blentinel\Hub\blentinel_hub.toml`, then restart the task: `Stop-ScheduledTask "Blentinel Hub"; Start-ScheduledTask "Blentinel Hub"`.

## 5. HTTPS (optional)

See [https.md](https.md).

## Upgrading

Blentinel is **pre-alpha** (DECISIONS D-30): upgrades may change the database schema without migrating old data. Check `CHANGELOG.md` for **Breaking** entries. If the new hub refuses to start with "this database was created before schema migrations existed":

1. Stop the hub.
2. Delete `blentinel.db`, `blentinel.db-wal` and `blentinel.db-shm` in `state_dir`.
3. Start the hub. Probe identities and the `[[probes]]` whitelist are kept; history starts fresh.

Within a schema version, the hub applies new migrations automatically at startup. Upgrade the **hub before the probes**.

## Backups

Back up `state_dir`: the key, the token and the database. For a consistent copy of the database while the hub runs, use `sqlite3 blentinel.db ".backup backup.db"`. A built-in `hub backup` command is planned (proposal 0004).
