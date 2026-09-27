# Installing a probe

A probe is one self-contained binary that runs inside a client network and pushes reports to the hub. It needs **outbound** access to the hub's URL and nothing inbound.

> **Pre-alpha.** Build from source ([CONTRIBUTING.md](../../CONTRIBUTING.md)). One-command installation with enrollment codes, without copying keys around, is planned (ROADMAP M2, proposal 0002).

## 1. Build for the target machine

```sh
./target/release/blentinelmake probe publish --target x86_64-unknown-linux-musl   # Linux x86_64
./target/release/blentinelmake probe publish --target aarch64-unknown-linux-musl  # Raspberry Pi 4/5
./target/release/blentinelmake probe publish --target x86_64-pc-windows-msvc      # Windows
```

Use the **musl** targets on Linux: the binary is fully static and runs on any distribution. Copy the resulting `app/` folder to the probe machine. Everything the probe creates (identity, buffer, log) stays in its own folder.

## 2. Configure

```sh
./probe --init          # writes a commented blentinel_probe.toml next to the binary
```

Set at least the company, the hub URL and the resources to check:

```toml
[agent]
company_id = "example-company"
hub_url = "http://192.0.2.10:3000"
interval = 60
site = "head-office"

[[resources]]
name = "Core switch"
type = "ping"
target = "192.0.2.1"
```

A complete example is in [`probe/blentinel_probe.example.toml`](../../probe/blentinel_probe.example.toml), and every option is in [reference/configuration.md](../reference/configuration.md).

## 3. First start and registration

Start the **hub first**. On its first run the probe fetches the hub's key (the handshake) before it prints its own ID, and retries every 2 minutes while the hub is unreachable.

```sh
./probe
```

On first run it creates its identity (`identity.key`) and prints its **public key**, which is its ID, plus a ready-made hub entry, then exits:

```
PUBLIC KEY (paste into blentinel_hub.toml):
a773d201237ea75c354a4c2e05325110ea6d4fee9f69c40f1c14b882b2a7dfcd
```

Add it to the hub's `blentinel_hub.toml`:

```toml
[[probes]]
name = "head-office"
public_key = "a773d201237ea75c354a4c2e05325110ea6d4fee9f69c40f1c14b882b2a7dfcd"
```

The hub picks up the change without a restart. Start the probe again; it now reports every `interval` seconds, and the company appears on the dashboard.

## 4. Run as a service

### Windows

The probe detects when the Service Control Manager starts it and runs as a service automatically. The package includes `install_probe_service.ps1`. Run it **as administrator** from the package folder:

```powershell
.\install_probe_service.ps1     # installs to C:\Blentinel\probe, keeps an existing config
```

It registers `BlentinelProbe` (automatic start, restart on failure) and keeps an existing `blentinel_probe.toml`. Complete the configuration and registration (steps 2–3) in `C:\Blentinel\probe` first. Logs: `blentinel_probe.log` in that folder.

### Linux (systemd)

From the unpacked package folder:

```sh
sudo ./install_probe_service.sh
```

- It installs to `/opt/blentinel-probe`, the probe's own folder, which also holds its identity, config, buffer and log. It creates the system user `blentinel` if needed.
- It enables the `blentinel-probe` service. On a **first install** it doesn't start it yet, because you still need to set the config and register the probe. It prints the next steps:
  1. Edit `/opt/blentinel-probe/blentinel_probe.toml`.
  2. Run `sudo -u blentinel /opt/blentinel-probe/probe` once to get the probe's ID, and add it to the hub's `[[probes]]`.
  3. `sudo systemctl start blentinel-probe`.
- **Upgrading:** run the same script from the new package. It replaces **only the binary**. The identity (`identity.key`), config and buffered reports are kept, so the probe keeps its ID.
- **Ping without root:** the service grants the probe `CAP_NET_RAW`, which survives upgrades. Only when you run the binary by hand as a normal user do you need `sudo setcap cap_net_raw+ep /opt/blentinel-probe/probe`.

Logs: `journalctl -u blentinel-probe -f` and `/opt/blentinel-probe/blentinel_probe.log`.

## When the hub is unreachable

The probe keeps checking and buffers undelivered reports in `blentinel_blackbox.db` (up to 7 days / 20,000 reports), then delivers them oldest-first once the hub is back. Delivered reports are deleted immediately. Reports the hub rejects for good (for example before the probe is registered) are dropped, not buffered. Details in [architecture.md](../architecture.md#probe).

## Files in the probe folder

| File | What it is |
|---|---|
| `probe` / `probe.exe` | The binary |
| `blentinel_probe.toml` | Configuration (hot-reloaded on change) |
| `identity.key` | The probe's Ed25519 private key. **Its identity: back it up, never share it.** If it's lost, the probe gets a new ID and must be registered again. |
| `blentinel_blackbox.db` | Buffered reports (reset automatically if the probe or hub key changes) |
| `blentinel_probe.log` | Log file |

Problems? See [guide/troubleshooting.md](../guide/troubleshooting.md).
