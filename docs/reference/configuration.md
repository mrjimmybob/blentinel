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
| `state_dir` | *required* | Directory for runtime state. Relative `db_path`, `identity_key_path` and `auth_token_path` are resolved against it. |
| `db_path` | *required* | Database file, e.g. `blentinel.db` |
| `identity_key_path` | `hub_identity.key` | Hub X25519 private key |
| `auth_token_path` | `hub_auth.token` | Admin token for the web UI |
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

### What reloads without a restart

Changes to `[[probes]]`, `probe_timeout_secs`, `[retention]` and `[alerts]` (including SMTP) apply immediately. Changes to `host`, `port`, `state_dir`, `db_path`, `identity_key_path`, `auth_token_path` and `[server.tls]` need a restart; the hub logs a warning when one of them changes.

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

Changes to `interval`, `company_id`, `site` and the resource list apply from the next cycle. **`hub_url` and `hub_public_key` are read only at startup**, so restart the probe after changing them. (Earlier docs said every probe setting reloads; that was never true for these two.)
