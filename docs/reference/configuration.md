# Configuration reference

The hub reads a TOML file and **reloads it automatically** when it changes: the new file is validated, and on success it replaces the old configuration, otherwise the error is logged and the old one kept. Generate a fully commented starting point with `hub --init`; the template is checked by tests, so it always parses.

The probe has **no config file**: its checks and report interval are managed on the hub and sent to it (D-49). Its own files are described [below](#probe-files).

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

| `behind_proxy` | `false` | The hub runs behind a TLS-terminating reverse proxy: session cookies are marked `Secure` and the client address comes from the last `X-Forwarded-For` entry. It trusts the proxy: never set it on a hub clients can reach directly. |

### `[server.tls]`

HTTPS is **on by default**, also when the section is missing (D-43).

| Key | Default | Description |
|---|---|---|
| `enabled` | `true` | Serve HTTPS. `false`: plain HTTP for everything (only behind a TLS proxy, or for testing) |
| `cert_path` | `hub_tls_cert.pem` | Certificate (PEM); a self-signed one is generated if missing. Relative to the **working directory**. |
| `key_path` | `hub_tls_key.pem` | Private key (PEM); generated if missing. Relative to the **working directory**. |
| `https_port` | `3443` | Where HTTPS listens. If it differs from `port` (split mode), `port` serves plain HTTP for the probe endpoints only and redirects everything else here. If it equals `port`: HTTPS only. |

See [install/https.md](../install/https.md).

### `[retention]`

| Key | Default | Description |
|---|---|---|
| `enabled` | `true` | Check the database size every 10 minutes |
| `warn_db_size_mb` | `1000` | Warn (log + admin page) above this size |
| `auto` | `false` | Archive automatically when over the size limit |
| `archive_older_than_days` | `90` | What "old" means for archiving |
| `archive_path` | `archives` | Where archive files go. Relative to the **working directory**. |

See [guide/data-retention.md](../guide/data-retention.md).

### `[backups]`

| Key | Default | Description |
|---|---|---|
| `path` | `backups` | The folder backups are written to: the daily backup, **Back up now**, `hub backup`, and the backup before a database upgrade. Relative to `state_dir`. Point it at another disk or a mounted share so backups outlive the server. |

When backups run and how many are kept is set on the admin page, not here (D-60). See [install/hub.md](../install/hub.md#backups).

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
| `probe_timeout_secs`, `[retention]`, `[alerts]`, `[backups]` | Used from the next report, sweep, alert or backup |
| `host`, `port`, `[server.tls]` | The hub stops listening on the old address (requests in flight get 5 s to finish) and starts on the new one. An address that is already in use refuses the reload. Turning TLS on creates the certificate if it's missing. |
| `state_dir`, `db_path` | The hub switches to the database at the new path, creating it if needed (an older one is backed up and upgraded, as at startup). **Data isn't moved**: an empty path means empty history, and no users (the hub creates a new setup code). |
| `identity_key_path` | The hub uses the key at the new path (created if missing). Probes enrolled with the old key can no longer deliver reports: enroll them again with new codes. |

## Probe files

All in the probe's folder, **next to the probe binary** (not the working directory). The probe writes them; nothing here is edited by hand.

### `config.json`

The checks and report interval the hub sent last (a `common::checks::ProbeConfig`: `revision`, `interval_secs`, `checks`). The probe runs it after a restart and while the hub is unreachable (FP-4), and replaces it, atomically, when the hub sends another. A damaged file is ignored: the probe checks nothing until the hub sends its checks again, with the answer to its next report. Checks are described in [reference/checks.md](checks.md); how they reach the probe in [reference/protocol.md](protocol.md#the-hubs-answer).

### `enrollment.json`

Written by `probe install --code <code>` (or `probe enroll <code>`) in the probe's folder; don't edit it. It holds the hub's address, its X25519 public key and Ed25519 signing key, the SHA-256 of its TLS certificate's public key (`tls_spki_sha256`, if the address is `https://` and the hub serves TLS itself), the hub's next pins while it rotates them (`next`, D-61), and the probe's name, company and site as the hub assigned them. Enrolling again replaces it, and a running probe follows the new enrollment from its next cycle. When the hub revokes the probe, the file is renamed `enrollment.revoked.json` and the probe waits for a new enrollment. See [install/probe.md](../install/probe.md).
