# Troubleshooting

The hub logs to stdout (under systemd: `journalctl -u blentinel-hub -f`). Rejected reports, alerts and errors are always logged; `--verbose` adds one line per accepted report, and `RUST_LOG` gives finer control ([reference/cli.md](../reference/cli.md)). The probe logs to `blentinel_probe.log` next to its binary.

## The hub rejects a probe's reports

Both sides log why. The hub logs `Rejected report: …` (or `Security: rejected report: …`). The probe logs the hub's answer, e.g. `Hub answered 403 Forbidden: report from unregistered probe a773d201 (unknown_probe)`. The codes are in [reference/protocol.md](../reference/protocol.md#responses-to-post-apireport).

| Hub log / status | Cause | Fix |
|---|---|---|
| `report from unregistered probe` · 403 | The probe's public key isn't in `[[probes]]` | Add it (the probe prints it on first run). No restart needed. |
| `invalid signature` · 403 | The key in `[[probes]]` doesn't match the probe's `identity.key`, e.g. after a reinstall created a new identity | Put the probe's current public key in `[[probes]]` |
| `decryption failed` · 401 | The probe sealed the report for a different hub key: the hub's `hub_identity.key` changed, or `hub_public_key` in the probe config is outdated | Set the probe's `hub_public_key` to the hub's current key (in `hub_identity.pub`); it reloads. Without `hub_public_key`, restart the probe so it fetches the key again. |
| `timestamp is in the future` · 422 | The probe's clock is more than 5 minutes ahead of the hub's | Sync clocks (NTP) on both machines |
| `older than any probe buffers reports` · 422 | A report older than 7 days: a replay, or a very stale buffer | Nothing: expected protection |
| `not newer than its latest stored report` · 409 | A replay or duplicate delivery | Nothing: the probe treats it as delivered |

Reports rejected with 400/401/403/422 are **dropped** by the probe, not buffered. Fix the cause; new reports then get through.

## A probe shows as expired, but it's running

- It can't reach the hub: check `hub_url`, DNS, firewall, and the probe log for `Hub unreachable`.
- Its reports are being rejected: see above.
- `probe_timeout_secs` is too short for its `interval`: use 2–3× the interval.

A probe becomes active again with its next accepted report.

## The probe never prints its ID on first run

On first start the probe fetches the hub's key before printing its ID, and retries every 2 minutes. Start the hub first, check `hub_url`, and wait.

## Ping checks always fail on Linux

ICMP needs privileges. The systemd service from `install_probe_service.sh` grants them (`AmbientCapabilities=CAP_NET_RAW`); an older unit file may not, so re-run the installer. When running the binary by hand: run as root, or `sudo setcap cap_net_raw+ep /path/to/probe` (lost whenever the binary is replaced).

## "Identity or hub key changed … Reinitializing"

The probe's or hub's key changed, so buffered reports can no longer be verified or opened. The buffer is cleared automatically; nothing to do.

## The hub won't start: "database was created before schema migrations existed"

Pre-alpha upgrade (D-30). Stop the hub, delete `blentinel.db`, `blentinel.db-wal` and `blentinel.db-shm`, and start it. See [install/hub.md](../install/hub.md#upgrading).

## HTTPS problems

See [install/https.md](../install/https.md#troubleshooting).

## No alert e-mails

- `[alerts] enabled = true`, `smtp.server` and `smtp.from` are set, and there's a recipient for that company.
- The hub log shows `ERROR Failed to send … alert: …` with the SMTP error.
- The resource may be silenced (UI or `GET /api/silences`).
- Alerts fire on **changes**: a resource that stays down sends one DOWN alert, not one per report.
