# Troubleshooting

**Start with `hub doctor`** (as root/Administrator): it checks the config, service, data folder, key, database, certificate, ports and disk space, and says what to do about each problem. `hub status` shows the service, ports, database and pending setup code.

**On a probe, `probe doctor`** (as root/Administrator) checks the service, its folder, identity and enrollment, whether the hub answers through its pinned certificate, and the clock against the hub's. `probe status` shows its hub, checks and reports waiting to be sent ([install/probe.md](../install/probe.md#checking-on-it)).

The hub logs to stdout (under systemd: `journalctl -u blentinel-hub -f`; the Windows service: `C:\ProgramData\Blentinel\Hub\hub.log`). Rejected reports, alerts and errors are always logged; `--verbose` adds one line per accepted report, and `RUST_LOG` gives finer control ([reference/cli.md](../reference/cli.md)). The probe logs to `blentinel_probe.log` in its folder (installed on Linux: also `journalctl -u blentinel-probe`).

## The hub rejects a probe's reports

Both sides log why. The hub logs `Rejected report: …` (or `Security: rejected report: …`). The probe logs the hub's answer, e.g. `Hub answered 403 Forbidden: report from unregistered probe a773d201 (enroll it with a code) (unknown_probe)`. The codes are in [reference/protocol.md](../reference/protocol.md#responses-to-post-apireport).

| Hub log / status | Cause | Fix |
|---|---|---|
| `report from unregistered probe` · 403 `unknown_probe` | The hub has no record of this probe: it was never enrolled with this hub, it was removed on the admin page, or the hub's database was reset | **Add probe** in the hub, then `probe enroll <code>` on the probe |
| `report from revoked probe` · 403 | Someone revoked it. The probe logs `The hub revoked this probe` and stops reporting (its `enrollment.json` becomes `enrollment.revoked.json`) | Enroll it again with a new code, if it should report |
| `invalid signature` · 403 | The report wasn't signed with the key the probe enrolled with, e.g. `identity.key` was replaced | Enroll the probe again |
| `decryption failed` · 401 | The probe sealed the report for a different hub key: the hub's `hub_identity.key` changed since the probe enrolled | Enroll the probe again with a new code |
| `timestamp is in the future` · 422 | The probe's clock is more than 5 minutes ahead of the hub's | Sync clocks (NTP) on both machines |
| `older than any probe buffers reports` · 422 | A report older than 7 days: a replay, or a very stale buffer | Nothing: expected protection |
| `not newer than its latest stored report` · 409 | A replay or duplicate delivery | Nothing: the probe treats it as delivered |

Reports rejected with 400/401/403/422 are **dropped** by the probe, not buffered. Fix the cause; new reports then get through.

## A probe won't enroll

`probe install --code` and `probe enroll` print why:

| Message | Cause | Fix |
|---|---|---|
| `not an enrollment code` / `the code is damaged or incomplete` | The code was cut or mistyped when copied, or it's the hub's 8-character **setup code** (that one only creates the first Owner) | Copy the whole enrollment code from **Add probe**, `BLT1-` included, in one piece |
| `invalid or expired enrollment code` · 403 `invalid_code` | Expired (24 h by default), already used up, revoked, or made by another hub | Create a new code with **Add probe** |
| `the hub's certificate doesn't match the one in the enrollment code` | The server at that address doesn't hold the certificate the code pinned: the hub's certificate changed after the code was made, or something is intercepting | Create a new code; if the certificate didn't change, investigate the network |
| connection refused / timed out | The probe can't reach the address in the code | Check the address (Add probe lets you change it), DNS and firewall |
| `the probe's clock is more than 5 minutes off` · 422 | Clock skew | Sync clocks (NTP) |

A probe that starts before it's enrolled logs `Not enrolled` and waits; it starts reporting once `enrollment.json` appears.

## Probe log warnings about the hub's answers

| Probe log | Cause | Fix |
|---|---|---|
| `This machine's clock is off the hub's` | The clocks differ by more than a minute. At 5 minutes the hub starts refusing reports (`timestamp_rejected`) | Sync clocks (NTP) on both machines |
| `The hub stored the report, but its answer can't be trusted` | The answer wasn't signed by the hub the probe enrolled with, or was for another report: a proxy changed the response, the hub's identity key was replaced, or someone is interfering | If `hub_identity.key` changed, enroll the probe again; otherwise check the network path |

## A probe shows as expired, but it's running

- It can't reach the hub: check the hub address it enrolled with (`enrollment.json`), DNS, firewall, and the probe log for `Hub unreachable`.
- Its reports are being rejected: see above.
- `probe_timeout_secs` is too short for its `interval`: use 2–3× the interval.

A probe becomes active again with its next accepted report.

## The probe never prints its ID on first run

On first start the probe fetches the hub's key before printing its ID, and retries every 2 minutes. Start the hub first, check `hub_url`, and wait.

## Ping checks always fail on Linux

ICMP needs privileges. The service `probe install` sets up grants them (`AmbientCapabilities=CAP_NET_RAW`); a probe installed by the old scripts may not have them, so run `probe install` again. When running the binary by hand: run as root, or `sudo setcap cap_net_raw+ep /path/to/probe` (lost whenever the binary is replaced).

## "Identity or hub key changed … Reinitializing"

The probe's or hub's key changed, so buffered reports can no longer be verified or opened. The buffer is cleared automatically; nothing to do.

## The hub won't start: "database was created before schema migrations existed"

Pre-alpha upgrade (D-30). Stop the hub, delete `blentinel.db`, `blentinel.db-wal` and `blentinel.db-shm`, and start it. See [install/hub.md](../install/hub.md#upgrading).

## The browser warns about the hub's certificate

Expected: the hub uses a self-signed certificate by default. Compare the SHA-256 fingerprint the browser shows with the `TLS: Certificate fingerprint` line in the hub log; if they match, accept it. To avoid the warning, trust `hub_tls_cert.pem` on the admins' machines or use your own certificate ([install/https.md](../install/https.md#your-browser-warns-about-the-certificate)).

## The web UI isn't reachable on port 3000

By default the web UI is only on HTTPS, port **3443** (`https_port`); port 3000 serves probes and redirects browsers. Open `https://<hub>:3443` and allow that port in the firewall.

## A script gets `403 csrf`

Every POST to the API needs the header `X-Blentinel: 1` ([reference/api.md](../reference/api.md#authentication-and-access)).

## Login says "too many failed attempts"

Wait for the time shown (at most 15 minutes), or restart the hub, which clears the counters. If you forgot the password: `hub user reset-password <name>` ([guide/users.md](users.md#locked-out-hub-user)).

## HTTPS problems

See [install/https.md](../install/https.md#troubleshooting).

## No alert e-mails

- `[alerts] enabled = true`, `smtp.server` and `smtp.from` are set, and there's a recipient for that company.
- The hub log shows `ERROR Failed to send … alert: …` with the SMTP error.
- The resource may be silenced (UI or `GET /api/silences`).
- Alerts fire on **changes**: a resource that stays down sends one DOWN alert, not one per report.
