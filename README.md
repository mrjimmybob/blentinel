# Blentinel

**Monitoring for Managed Service Providers that works from inside any client network, without opening a single inbound port.**

Blentinel (the *Bl*ind S*entinel*) has two parts. A small **probe** runs inside each client network and checks the devices there: servers, switches, NAS, printers, web services, disks, CPU. It **pushes** signed, encrypted reports out to your central **hub**, which shows every client on one dashboard and alerts you when something breaks.

![Company view: uptime chart, probe status and device checks](images/blentinel.png)

> **Status: pre-alpha.** Usable, and used in production by its author, but not stable yet. Upgrades may require resetting the hub's database. See [CHANGELOG.md](CHANGELOG.md) before upgrading.

## Why push?

Most monitoring tools reach *into* the networks they watch, so every client network needs inbound firewall rules, a VPN or a tunnel. That's more attack surface and more setup per client.

A Blentinel probe only makes **outbound** requests to the hub, the same as a browser. It works behind NAT, carrier-grade NAT and strict firewalls. The hub never connects to a probe.

## How it works

```
 client network                                     your hub
 ┌────────────────────────────┐                     ┌───────────────────────┐
 │ probe ── ping/http/tcp ──► devices               │  verify · store ·     │
 │   │  ── cpu/mem/disk ────► its own host          │  alert · dashboard    │
 │   └─ signed + encrypted report ──── HTTPS/HTTP ─►│                       │
 └────────────────────────────┘   (outbound only)   └───────────────────────┘
```

- **Probe:** one self-contained binary (Linux or Windows) with no runtime or dependencies to install, and no config file. It runs the checks the hub gives it every *N* seconds, signs each report with its own key, encrypts it for the hub, and buffers reports while the hub is unreachable.
- **Hub:** accepts reports only from enrolled probes with valid signatures, decides which company and site each probe reports for, rejects replays, stores the history, tracks which probes have gone silent, sends e-mail alerts, and serves the web dashboard.

More detail: [docs/architecture.md](docs/architecture.md) · [docs/security.md](docs/security.md)

## Features today

- **Checks:** ping (latency), HTTP(S), TCP port, and CPU, memory, disk and uptime of the probe's machine. Checks are records on the hub, sent to each probe in the hub's signed answer to its reports; nothing is edited on the client's machine
- **Dashboard:** companies sorted by severity (critical / degraded / healthy), per-probe device status, uptime history (24 h / 7 d / 30 d / all)
- **Alerts** (e-mail): device down and recovered, disk/CPU/memory thresholds, probe stopped reporting; per-company recipients and thresholds; silences
- **Security:** Ed25519-signed and X25519/ChaCha20-Poly1305-encrypted reports, enrollment with single-use codes, replay protection, HTTPS for the web UI by default, HTTPS for probes pinned to the hub's certificate, CSRF protection and login rate limiting
- **Resilience:** probes buffer reports for up to 7 days while the hub is down, then deliver them in order
- **Companies and sites:** added, named, given contacts and addresses, or archived in the UI; probes are added to a site with **Add probe** and a one-time code
- **Checks in the UI:** each probe's page lists its checks with their latest results; add, edit, disable or delete them, copy them from another probe, set the report interval, and see whether the probe runs the latest ones
- **Users:** personal accounts with roles (Owner, Admin, Technician, Viewer, Client), a first-run setup wizard; each user sees all companies, only some, or all except some, so technicians and customers get exactly their companies; an audit log of who did what
- **Data:** embedded SQLite (nothing to install), retention warnings and archiving

Coming next, in order: one-command probe installation, smarter alerting, probe auto-update, more check types (TLS expiry, backups, services, SNMP), and reports. See the [roadmap](docs/ROADMAP.md).

## Getting started

There are no pre-built binaries yet, so build from source first ([CONTRIBUTING.md](CONTRIBUTING.md#set-up)). Then:

1. **Set up the hub**, then open `https://<hub>:3443` and create the first Owner with the setup code it prints: [docs/install/hub.md](docs/install/hub.md)
2. **Add a company and site, then install a probe in that network and enroll it with a code from the hub:** [docs/install/probe.md](docs/install/probe.md)
3. Optional: **probes over HTTPS, your own certificate, or a reverse proxy:** [docs/install/https.md](docs/install/https.md)

Running into problems? [docs/guide/troubleshooting.md](docs/guide/troubleshooting.md)

## Documentation

| | |
|---|---|
| **Guides** | [Users & roles](docs/guide/users.md) · [Companies & sites](docs/guide/companies-sites.md) · [Audit log](docs/guide/audit.md) · [Alerting](docs/guide/alerting.md) · [Data retention & archives](docs/guide/data-retention.md) · [Troubleshooting](docs/guide/troubleshooting.md) |
| **Reference** | [Configuration](docs/reference/configuration.md) · [Checks](docs/reference/checks.md) · [CLI](docs/reference/cli.md) · [HTTP API](docs/reference/api.md) · [Probe protocol](docs/reference/protocol.md) · [Database](docs/reference/database.md) |
| **Design** | [Architecture](docs/architecture.md) · [Security](docs/security.md) · [Principles](docs/PRINCIPLES.md) · [Decisions](docs/DECISIONS.md) · [Proposals](docs/proposals/) |
| **Project** | [Roadmap](docs/ROADMAP.md) · [Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) · [Engineering standards](docs/ENGINEERING.md) |

A PDF manual can be generated from these pages (`blentinelmake docs`, see [CONTRIBUTING.md](CONTRIBUTING.md#manual)).

## Principles

Blentinel's design is bound by a few rules that don't change ([docs/PRINCIPLES.md](docs/PRINCIPLES.md)):

1. **Push only:** probes connect out; the hub never connects to a probe.
2. **Zero-dependency probe:** one static binary; everything it needs lives in its own folder.
3. **Zero trust:** every report is signed and encrypted; the hub trusts nothing a probe can't prove.
4. **Resilient:** a probe keeps working and buffering while the hub is unreachable.
5. **Easy to install and run:** embedded database, no external services; managed from the web UI (in progress).

## Contributing

Build instructions, tests and how changes are made: [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
