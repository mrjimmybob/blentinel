# Checks reference

A **check** is one thing one probe looks at: a ping, a web page, a port, a disk. Checks are records on the hub (M2-04, D-49). The probe has no config file: the hub sends each probe its enabled checks and its report interval in the signed answer to a report, and the probe runs them from then on, also after a restart or while the hub is unreachable ([protocol.md](protocol.md#the-probes-checks-config)).

Every check has a stable **ID** the hub gives it. Results, alert state and silences belong to that ID, so renaming a check or changing its target keeps its history, its alerts and its silence.

Checks are managed on the **probe's page** in the web UI (click a probe's name on its company's pages): add, copy from another probe, edit, disable, delete, the report interval, whether the probe runs them, and **Run checks now** (a command the probe gets with the answer to its next report: [protocol.md](protocol.md#commands-commands)). Seeing it takes any role that sees the probe; changing it, Technician or above within the user's companies. Scripts can use the API: [api.md](api.md#checks).

## A check

```json
{ "name": "Core switch", "spec": { "kind": "ping", "host": "192.0.2.1" }, "enabled": true }
```

| Field | Rules |
|---|---|
| `name` | 1–100 characters (spaces at the ends are trimmed) |
| `spec` | What to check: a `kind` and its settings (below) |
| `enabled` | Default `true`. A disabled check stays on the hub but isn't sent to the probe. |

Validation is one piece of code (`common::checks::CheckSpec::validate`), run by the hub's API and again by the probe on every config it receives. A probe runs at most 500 checks, and reports every 5–3600 seconds (`interval_secs` of the probe, default 60).

## Kinds

The probe only measures: it reports whether the check succeeded, a message, and the metrics below. The first metric is the headline value the UI shows. Thresholds are the hub's business ([guide/alerting.md](../guide/alerting.md)). Every check is limited to 30 seconds, so a slow one can't hold up the report.

| `kind` | Settings | Up when | Latency | Metrics |
|---|---|---|---|---|
| `ping` | `host`: an IP address | an ICMP echo is answered | ✓ | |
| `http` | `url`: `http://…` or `https://…` | a `GET` answers 2xx within 5 s | ✓ | |
| `tcp` | `host`: a name or IP address; `port`: 1–65535 | the port accepts a connection within 3 s | ✓ | |
| `cpu` | none | always (it measures) | | `usage` % |
| `memory` | none | always | | `usage` %, `used` GB, `total` GB |
| `disk` | `path` (optional): a mount point such as `/`, `/data`, `C:\` or `C:`; without it, the system disk | the disk is found | | `usage` %, `used` GB, `free` GB, `total` GB |
| `uptime` | none | always | | `uptime` s |

Examples:

```json
{ "kind": "ping", "host": "192.168.1.1" }
{ "kind": "http", "url": "https://example.com/health" }
{ "kind": "tcp", "host": "db.example.lan", "port": 5432 }
{ "kind": "disk", "path": "/data" }
{ "kind": "cpu" }
```

`cpu`, `memory`, `disk` and `uptime` look at the machine the probe runs on. `ping` needs raw sockets: root or `CAP_NET_RAW` on Linux (the systemd unit grants it), administrator rights not needed on Windows.

The hub's `[alerts.thresholds]` apply to the `usage` metric of `disk`, `cpu` and `memory` checks.

## How a change reaches the probe

1. You add, change, disable or delete a check, or change the probe's interval. The probe's config gets a new **revision** (a digest of its content).
2. The probe's next report says which revision it runs. It differs, so the hub's answer carries the new config.
3. The probe validates it, saves it to `config.json`, and runs it at once. Its next report carries the new revision and results for the new checks.

So a change takes effect within one report interval. `GET /api/probes/{id}/checks` shows `sync`: `in_sync`, `pending` (on its way), or `rejected` with the probe's reason, e.g. a check kind that this probe version doesn't know. A refused config isn't sent again: the probe keeps running its previous one until the hub has another. Results from a check the probe no longer has, or from another probe's check, are ignored.

More kinds (TLS certificate, DNS, services, SNMP, …) are planned: [proposal 0006](../proposals/0006-check-catalogue.md).
