# Configuration reference

Both programs read a TOML file and **reload it automatically** when it changes: the new file is validated, and on success it replaces the old configuration, otherwise the error is logged and the old one kept.

Generate a fully commented starting point with `hub --init` / `probe --init`. Those templates and [`probe/blentinel_probe.example.toml`](../../probe/blentinel_probe.example.toml) are checked by tests, so they always parse.

## Hub: `blentinel_hub.toml`

Location: the working directory, or `hub --config <path>`.

### `[server]`

| Key | Default | Description |
|---|---|---|
| `host` | *required* | Address to listen on, e.g. `0.0.0.0` |
| `port` | *required* | HTTP port (HTTPS port when TLS is on without `https_port`) |
| `state_dir` | *required* | Directory for runtime state. Relative `db_path` and `identity_key_path` are resolved against it. The first-run `setup.code` is written here. |
| `db_path` | *required* | Database file, e.g. `blentinel.db` |
| `identity_key_path` | `hub_identity.key` | Hub X25519 private key |
| `probe_timeout_secs` | `300` | Seconds without a report before a probe is **expired**. Use 2–3× the probes' `interval`. |

### `[server.tls]` (optional)

| Key | Default | Description |
|---|---|---|
| `enabled` | `false` | Serve HTTPS |
| `cert_path` | `hub_tls_cert.pem` | Certificate (PEM); generated if missing. Relative to the **working directory**. |
| `key_path` | `hub_tls_key.pem` | Private key (PEM); generated if missing. Relative to the **working directory**. |
| `https_port` | none | If set, HTTP stays on `port` and HTTPS listens here; if not, HTTPS replaces HTTP |

See [install/https.md](../install/https.md).

### `[[probes]]`: the probe whitelist

One entry per probe allowed to report:

| Key | Description |
|---|---|
| `name` | Display name |
| `public_key` | The probe's ID: 64 hex characters (its Ed25519 public key), printed by the probe on first run |

### `[retention]`

| Key | Default | Description |
|---|---|---|
| `enabled` | `true` | Check the database size every 10 minutes |
| `warn_db_size_mb` | `1000` | Warn (log + admin page) above this size |
| `auto` | `false` | Archive automatically when over the size limit |
| `archive_older_than_days` | `90` | What "old" means for archiving |
| `archive_path` | `archives` | Where archive files go. Relative to the **working directory**. |

See [guide/data-retention.md](../guide/data-retention.md).

### `[alerts]`

| Key | Default | Description |
|---|---|---|
| `enabled` | `false` | Send e-mail alerts. Requires `smtp.server` and `smtp.from`. |
| `default_recipients` | `[]` | Who gets alerts (unless a company override applies) |

`[alerts.thresholds]`: alert when a local metric reaches the limit (percent).

| Key | Default |
|---|---|
| `disk_percent` | `90` |
| `cpu_percent` | `95` |
| `mem_percent` | `90` |

`[alerts.smtp]`

| Key | Default | Description |
|---|---|---|
| `server` | `""` | SMTP host (required when alerts are enabled) |
| `port` | `587` | SMTP port (STARTTLS) |
| `username`, `password` | `""` | SMTP login (the password is stored in plain text in this file) |
| `from` | `""` | Sender address (required when alerts are enabled) |

`[alerts.company_overrides."<company_id>"]`: other recipients and thresholds for one company:

```toml
[alerts.company_overrides."example-company"]
alert_emails = ["ops@example.com"]

[alerts.company_overrides."example-company".thresholds]
disk_percent = 85
cpu_percent = 90
mem_percent = 85
```

`[[alerts.technicians]]` (`name`, `email`, optional `phone`) is accepted but not used yet.

See [guide/alerting.md](../guide/alerting.md).

### Reloading

**Every setting applies without a restart** (D-38). Save the file and the hub reloads it within a second. A reload is all or nothing: the file is validated, and everything it needs (the database, the key, the new address) is prepared first. If anything fails, the hub logs `Config reload failed; nothing changed: …` and keeps running as before. On success it logs `Configuration reloaded: …` with what changed.

| Setting | What happens on change |
|---|---|
| `[[probes]]`, `probe_timeout_secs`, `[retention]`, `[alerts]` | Used from the next report, sweep or alert |
| `host`, `port`, `[server.tls]` | The hub stops listening on the old address (requests in flight get 5 s to finish) and starts on the new one. An address that is already in use refuses the reload. Turning TLS on creates the certificate if it's missing. |
| `state_dir`, `db_path` | The hub switches to the database at the new path, creating it if needed. **Data isn't moved**: an empty path means empty history, and no users (the hub creates a new setup code). |
| `identity_key_path` | The hub uses the key at the new path (created if missing). Probes that fetched the old key by handshake can no longer deliver reports: set their `hub_public_key` to the new key (reloaded), or restart them. |

## Probe: `blentinel_probe.toml`

Location: **next to the probe binary** (not the working directory).

### `[agent]`

| Key | Default | Description |
|---|---|---|
| `company_id` | *required* | Which client this probe belongs to (shown on the dashboard) |
| `hub_url` | *required* | The hub, e.g. `http://192.0.2.10:3000` or `https://…:3443` |
| `interval` | *required* | Seconds between reports (> 0) |
| `site` | `main` | Location within the company |
| `hub_public_key` | none | The hub's X25519 public key (hex; `hub --print-public-key`). If set, the probe uses it instead of fetching it at startup. |

### `[[resources]]`: what to check

Each entry has a `name`, a `type` and a `target`:

| `type` | `target` | Reports |
|---|---|---|
| `ping` | IP address or host | ICMP echo; latency. On Linux needs root or `cap_net_raw`. |
| `http` | URL | GET request; any 2xx is up; latency |
| `tcp` | `host:port` | Whether the port accepts a connection (3 s timeout); latency |
| `local_cpu` | `localhost` | CPU usage % of the probe's machine |
| `local_mem` | `localhost` | Memory usage % |
| `local_disk` | mount point (`/`, `C:\`) or `auto` | Disk usage % (`auto`: the system drive) |
| `local_uptime` | `localhost` | Uptime of the probe's machine |
| `local_data` | `localhost` | Shortcut for CPU + memory + system disk + uptime |

**Every setting applies from the next cycle, without a restart** (D-38): `interval`, `company_id`, `site`, the resource list, `hub_url` and `hub_public_key`. Setting `hub_public_key` pins the hub key. Removing it makes the probe fetch the key from `hub_url` (handshake). If the new `hub_url` or key can't be used (no `http://`/`https://`, a malformed key, or a hub that doesn't answer), the probe keeps its current connection, logs `Can't use the new hub settings, keeping the current connection`, and tries again each cycle. A file that fails validation is ignored, and the previous settings stay.
