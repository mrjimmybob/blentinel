# Checks reference

A **check** is one thing one probe looks at: a ping, a web page, a port, a disk. Checks are records on the hub (M2-04, D-49). The probe has no config file: the hub sends each probe its enabled checks and its report interval in the signed answer to a report, and the probe runs them from then on, also after a restart or while the hub is unreachable ([protocol.md](protocol.md#the-probes-checks-config)).

Every check has a stable **ID** the hub gives it. Results, alert state and silences belong to that ID, so renaming a check or changing its target keeps its history, its alerts and its silence.

Checks are managed on the **probe's page** in the web UI (click a probe's name on its company's pages): add, copy from another probe, edit, disable, delete, the report interval, whether the probe runs them, and **Run checks now** (a command the probe gets with the answer to its next report: [protocol.md](protocol.md#commands-commands)). Seeing it takes any role that sees the probe; changing it, Technician or above within the user's companies. Scripts can use the API: [api.md](api.md#checks).

## A check

```json
{ "name": "Core switch", "spec": { "kind": "ping", "host": "192.0.2.1" }, "enabled": true,
  "fail_after": null, "recover_after": null, "limits": { "warning": 200 } }
```

| Field | Rules |
|---|---|
| `name` | 1–100 characters (spaces at the ends are trimmed) |
| `spec` | What to check: a `kind` and its settings (below) |
| `enabled` | Default `true`. A disabled check stays on the hub but isn't sent to the probe. |
| `limits` | The check's own thresholds for its metric: `{"warning": …, "down": …}`, each a number, `"off"`, or left out (inherited from its site, company or the hub; [alerting](../guide/alerting.md#thresholds), D-64). Percent for `cpu`, `memory`, `disk`; milliseconds for `ping`, `http`, `tcp`; days left for `tls` (the warning limit is the higher one); hours for `file` (the age of its newest file); none for `uptime`. |
| `fail_after`, `recover_after` | Failed / good results in a row before it alerts / counts as recovered, 1–10; `null` (the default): 2 ([alerting](../guide/alerting.md#several-results-in-a-row-flap-suppression), D-63). Hub-only: not part of the probe's config. |
| `depends_on` | The ID of the check of the same probe it is behind (the gateway's ping): while that one is Down, this one is Unknown and doesn't alert ([alerting](../guide/alerting.md#dependencies), D-68). `null` (the default): none. Hub-only. |

Validation is one piece of code (`common::checks::CheckSpec::validate`), run by the hub's API and again by the probe on every config it receives. A probe runs at most 500 checks, and reports every 5–3600 seconds (`interval_secs` of the probe, default 60).

## Kinds

The probe only measures: it reports whether the check succeeded, a message, and the metrics below. The first metric is the headline value the UI shows. Thresholds are the hub's business ([guide/alerting.md](../guide/alerting.md)). Every check is limited to 30 seconds, so a slow one can't hold up the report.

| `kind` | Settings | Up when | Latency | Metrics |
|---|---|---|---|---|
| `ping` | `host`: an IP address | an ICMP echo is answered | ✓ | |
| `http` | `url`: `http://…` or `https://…`; optional: `method`, `status`, `contains`, `absent`, `follow_redirects`, `headers`, `verify` ([below](#web-pages)) | the answer comes within 5 s with an expected status (2xx unless `status` says otherwise) and the expected text | ✓ | |
| `tcp` | `host`: a name or IP address; `port`: 1–65535 | the port accepts a connection within 3 s | ✓ | |
| `tls` | `host`: a name or IP address; `port`: 1–65535 (the form: empty is 443); `verify` (optional, default `true`) | the server's certificate hasn't expired and, with `verify`, a client would accept it ([below](#tls-certificates)) | ✓ | `days_left` d |
| `cpu` | none | always (it measures) | | `usage` % |
| `memory` | none | always | | `usage` %, `used` GB, `total` GB |
| `disk` | `path` (optional): a mount point such as `/`, `/data`, `C:\` or `C:`; without it, the system disk | the disk is found | | `usage` %, `used` GB, `free` GB, `total` GB |
| `file` | `path`: a file or a folder, as a full path; optional: `pattern`, `recursive`, `min_bytes` ([below](#files-and-folders)) | a file is there (in a folder: one that matches) and its newest isn't smaller than `min_bytes` | | `age` h, `size` MB, `files` |
| `uptime` | none | always | | `uptime` s |

Examples:

```json
{ "kind": "ping", "host": "192.168.1.1" }
{ "kind": "http", "url": "https://example.com/health" }
{ "kind": "http", "url": "https://intranet.example.lan/login", "status": "200, 401", "contains": "Sign in", "verify": false }
{ "kind": "tcp", "host": "db.example.lan", "port": 5432 }
{ "kind": "tls", "host": "www.example.com", "port": 443 }
{ "kind": "tls", "host": "nas.example.lan", "port": 5001, "verify": false }
{ "kind": "disk", "path": "/data" }
{ "kind": "file", "path": "D:\\Backups\\SQL", "pattern": "*.bak", "min_bytes": 524288000 }
{ "kind": "cpu" }
```

`cpu`, `memory`, `disk`, `file` and `uptime` look at the machine the probe runs on. `ping` needs raw sockets: root or `CAP_NET_RAW` on Linux (the systemd unit grants it), administrator rights not needed on Windows.

Thresholds apply to the `usage` metric of `disk`, `cpu` and `memory` checks, to the latency of `ping`, `http` and `tcp` checks, to the `age` of `file` checks, and to the `days_left` of `tls` checks. The probe only says whether a check worked; the hub decides Warning and Down ([alerting](../guide/alerting.md#statuses)).

### Files and folders

A `file` check (M5-03, D-79) answers "did last night's backup write a file, and is it a real one?". It looks at one file, or at the **newest** file of a folder, and reports how old it is (`age`, hours since it was last written), how big (`size`, MB) and how many files match (`files`).

| Setting | Default | Meaning |
|---|---|---|
| `path` | required | A file, or a folder. A full path on the probe's machine, written as that system writes it: `/var/backups`, `D:\Backups`, `\\nas\backups\sql` |
| `pattern` | every file | In a folder, the file names that count: `*.bak`, `backup-*.zip`, `db_??.dump` (`*`: any characters, `?`: any one; letter case doesn't matter). Not used when `path` is one file |
| `recursive` | `false` | Look in the folder's subfolders too (16 deep at most; folders reached through links aren't followed) |
| `min_bytes` | none | The newest file must be at least this big, in bytes (the form takes MB). A backup of a few bytes is a failed backup |

| What the probe finds | The check is | Message |
|---|---|---|
| a file | Up, then Warning or Down as it gets old (the hub's thresholds on `age`: by default a warning at 26 hours, down at 50) | `nightly.bak: 5.2 hours old, 1.4 GB (the newest of 14 files)` |
| a newest file smaller than `min_bytes` | Down | `nightly.bak: 5.2 hours old, 12 KB; smaller than 500 MB` |
| no file, or none that matches | Down | `No file matches *.bak` |
| a path it can't read | Down | `Can't read D:\Backups: …` with the system's reason |

The defaults suit a nightly backup: 26 hours is a night that ran late, 50 is two nights missed. For a weekly one, set the check's own limits (170 and 340 hours, say), or the site's or company's.

The probe reads names, sizes and dates only, never a file's content, and it only reads: nothing is created, changed or deleted. It must be allowed to list the folder: on Linux it runs as root; on Windows as the Local System account, which **can't read most network shares**. For a share, check the file on the machine that holds it, or give that machine's account access to the share. A check looks at 200,000 entries at most and says so when it stopped there (`the newest of the first … files`).

The newest file's **name** is part of the result, so it is visible on the hub to everyone who sees the probe.

### Web pages

An `http` check with only a `url` is a `GET` that must answer 2xx. The rest says what else to send and expect (M5-02, D-78); each is left out when it is the default:

| Setting | Default | Meaning |
|---|---|---|
| `method` | `"get"` | `"get"`, or `"head"`: headers only, lighter for a big page (and then no text to look for) |
| `status` | `"200-299"` | The status codes that are fine: codes and ranges separated by commas, e.g. `"200, 301-308, 401"` (100–599, at most 20). A login page that answers 401 is up with `"401"` |
| `contains` | none | Text the page must contain, exactly as written (case matters), e.g. `"Sign in"`. At most 200 characters |
| `absent` | none | Text the page must not contain, e.g. `"Maintenance"`: for a site that answers 200 with an error page |
| `follow_redirects` | `true` | Follows up to 10 redirects and judges the last answer. `false`: the redirect is the answer; expect its status (`"301-308"`) |
| `headers` | none | Up to 10 `{"name": …, "value": …}` sent with the request, e.g. `Accept-Language`, or `Host` for a site behind a load balancer. **Not for passwords or tokens:** they are stored and shown as written, to everyone who sees the check |
| `verify` | `true` | Over HTTPS, the certificate must be one a browser accepts ([who is trusted](#tls-certificates)). `false`: any certificate will do (a self-signed device, a private authority) |

The probe looks for text in the first 1 MiB of the page, read as UTF-8. A page a browser builds with JavaScript doesn't contain that text when the probe fetches it.

Messages: `HTTP 200 OK`; `HTTP 503 Service Unavailable (expected 200-299)`; `HTTP 200 OK, but the page doesn't contain "Sign in"`; `Connection failed: …` with the reason (refused, a timeout, a certificate that isn't trusted).

**How slow is too slow** is a threshold, not a setting of the check: its latency limits (`limits`, or the site's, company's or hub's; [alerting](../guide/alerting.md#thresholds)) make a slow page Warning or Down.

There is no login yet (basic authentication, tokens): it waits for a place to keep a check's secrets on the hub. To watch a protected page, expect its `401`.

### TLS certificates

A `tls` check (M5-01, D-77) answers "when does this certificate expire, and would a browser accept it?". The probe makes a real TLS handshake with `host:port` (TLS 1.2 or 1.3, with `host` as the server name) and reads the certificate the server presents. Any service that starts with TLS works: HTTPS (443), LDAPS (636), SMTPS (465), IMAPS (993), a firewall's or NAS's admin page. A service that starts in plain text and upgrades later (STARTTLS on port 25 or 587, RDP on 3389) doesn't: the handshake fails.

| The certificate | The check is | Message |
|---|---|---|
| valid and accepted | Up, then Warning or Down as its days run out (the hub's thresholds: by default a warning with 21 days left, down with 7) | `Certificate expires in 43 days (2026-11-23), issued by R11` |
| expired | Down, always | `Certificate expired 3 days ago (2026-10-08)` |
| not one a client would accept: its issuer isn't a public authority (self-signed, a private one), or it isn't for `host` | Down, unless `verify` is `false` | `Certificate not trusted: it isn't for www.example.com (expires 2026-11-23)` |
| none: the port is closed, or doesn't speak TLS | Down | `Connection failed: …`, `TLS handshake failed: …` |

`days_left` is reported whenever a certificate was read, also when the check is Down. It counts to the certificate's last moment, with decimals (`12.5`); the message rounds down to whole days.

**Who is trusted.** The probe carries Mozilla's list of public certificate authorities, the one browsers use, compiled into its binary. It never reads the machine's certificate store (FP-2), so a company's own authority installed on that machine isn't trusted. The same list decides for `http` checks, and for a probe whose hub is behind a proxy with a public certificate. For those, and for the self-signed certificates of printers, switches, NAS boxes and firewalls, set `verify` to `false` (in the form: **Only watch the expiry date**): the check then looks at the dates only. The list is updated with the probe.

The check doesn't look at revocation, at how strong the keys or the chain's signatures are, or at the intermediates' own expiry dates beyond what accepting the chain needs.

## How a change reaches the probe

1. You add, change, disable or delete a check, or change the probe's interval. The probe's config gets a new **revision** (a digest of its content).
2. The probe's next report says which revision it runs. It differs, so the hub's answer carries the new config.
3. The probe validates it, saves it to `config.json`, and runs it at once. Its next report carries the new revision and results for the new checks.

So a change takes effect within one report interval. `GET /api/probes/{id}/checks` shows `sync`: `in_sync`, `pending` (on its way), or `rejected` with the probe's reason, e.g. a check kind that this probe version doesn't know. A refused config isn't sent again: the probe keeps running its previous one until the hub has another. Results from a check the probe no longer has, or from another probe's check, are ignored.

More kinds (DNS, services, SNMP, …) are planned: [proposal 0006](../proposals/0006-check-catalogue.md).
