# Checks reference

A **check** is one thing one probe looks at: a ping, a web page, a port, a disk. Checks are records on the hub (M2-04, D-49). The probe has no config file: the hub sends each probe its enabled checks and its report interval in the signed answer to a report, and the probe runs them from then on, also after a restart or while the hub is unreachable ([protocol.md](protocol.md#the-probes-checks-config)).

Every check has a stable **ID** the hub gives it. Results, alert state and silences belong to that ID, so renaming a check or changing its target keeps its history, its alerts and its silence.

Checks are managed on the **probe's page** in the web UI (click a probe's name on its company's pages): add (one at a time, the ones the hub [suggests](#suggested-checks), or a [template](#templates)), copy from another probe, edit, disable, delete, the report interval, whether the probe runs them, and **Run checks now** (a command the probe gets with the answer to its next report: [protocol.md](protocol.md#commands-commands)). Seeing it takes any role that sees the probe; changing it, Technician or above within the user's companies. Scripts can use the API: [api.md](api.md#checks).

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
| `limits` | The check's own thresholds for its metric: `{"warning": …, "down": …}`, each a number, `"off"`, or left out (inherited from its site, company or the hub; [alerting](../guide/alerting.md#thresholds), D-64). Percent for `cpu`, `memory`, `disk`, `disks`; milliseconds for `ping`, `http`, `tcp`, `dns`, `database`, `snmp`; days left for `tls` (the warning limit is the higher one); hours for `file` (the age of its newest file); none for `uptime` and `service`. |
| `fail_after`, `recover_after` | Failed / good results in a row before it alerts / counts as recovered, 1–10; `null` (the default): 2 ([alerting](../guide/alerting.md#several-results-in-a-row-flap-suppression), D-63). Hub-only: not part of the probe's config. |
| `secrets` | What the check needs that nobody may read back: `{"community": "…"}` for an `snmp` check ([below](#secrets)). Write-only: one given replaces the one kept; left out or empty keeps it. A check's answer lists the names it has in `secrets_set`, never the values |
| `depends_on` | The ID of the check of the same probe it is behind (the gateway's ping): while that one is Down, this one is Unknown and doesn't alert ([alerting](../guide/alerting.md#dependencies), D-68). `null` (the default): none. Hub-only. |

Validation is one piece of code (`common::checks::CheckSpec::validate`), run by the hub's API and again by the probe on every config it receives. A probe runs at most 500 checks, and reports every 5–3600 seconds (`interval_secs` of the probe, default 60).

## Kinds

The probe only measures: it reports whether the check succeeded, a message, and the metrics below. The first metric is the headline value the UI shows. Thresholds are the hub's business ([guide/alerting.md](../guide/alerting.md)). Every check is limited to 30 seconds, so a slow one can't hold up the report.

| `kind` | Settings | Up when | Latency | Metrics |
|---|---|---|---|---|
| `ping` | `host`: an IP address, or a name the probe's machine resolves; optional: `count` (1–20, default 4), `tcp_port` ([below](#ping)) | at least one echo is answered, or else `tcp_port` accepts a connection | ✓ (the average) | `loss` %, `latency_max` ms, `jitter` ms |
| `http` | `url`: `http://…` or `https://…`; optional: `method`, `status`, `contains`, `absent`, `follow_redirects`, `headers`, `verify` ([below](#web-pages)) | the answer comes within 5 s with an expected status (2xx unless `status` says otherwise) and the expected text | ✓ | |
| `tcp` | `host`: a name or IP address; `port`: 1–65535 | the port accepts a connection within 3 s | ✓ | |
| `database` | `engine`: `postgres`, `mysql`, `mssql` or `mongodb`; `host`; optional: `port` (default: the engine's) ([below](#databases)) | the server answers the greeting of its own protocol and takes connections | ✓ | |
| `mail` | `protocol` (`smtp`, `imap`, `pop3`), `host`, `port` (optional), `security` (`starttls`, `tls`, `none`), `verify` ([below](#mail-servers)) | the server greets in its protocol, is encrypted as asked, and its certificate is good | ✓ | `days_left` of the certificate (encrypted connections) |
| `ntp` | `host`: a time server; `port` (optional, default 123) ([below](#time-servers)) | the server answers with a time it stands for | ✓ | `offset` s between this machine's clock and the server's, `stratum` |
| `nut` | `host`: the UPS server; `port` (optional, default 3493); `ups` (optional): the UPS's name there ([below](#ups)) | the server tells of the UPS, and its battery isn't low | ✓ | `charge` %, `runtime` min, `load` %, `warnings` |
| `tls` | `host`: a name or IP address; `port`: 1–65535 (the form: empty is 443); `verify` (optional, default `true`) | the server's certificate hasn't expired and, with `verify`, a client would accept it ([below](#tls-certificates)) | ✓ | `days_left` d |
| `dns` | `name`; optional: `record` (`a`, `aaaa`, `cname`, `mx`, `txt`, `ns`; default `a`), `server`, `expect` ([below](#dns)) | the name has a record of that type and, with `expect`, every answer is one of those | ✓ (the answer's time) | `answers` |
| `snmp` | `host`; `oids`: 1–20 values, each `oid` and optionally how to read it (`name`, `expect`, `not`, `walk`, `rows`, `of`, `rate`, `scale`, `unit`, `names`, `warn`, `down`); optional: `port` (default 161), `version` (`v2c`, `v3`), and for v3 `user`, `auth`, `privacy` ([below](#snmp)) | the agent answers with every value, each one what `expect` and `not` allow | ✓ (the answer's time; and each value's own `warn` and `down`) | one per number, named by its `name` or OID (a table's rows: `name: row`), at most 100; `answers` |
| `cpu` | none | always (it measures) | | `usage` % |
| `memory` | none | always | | `usage` %, `used` GB, `total` GB |
| `disk` | `path` (optional): a mount point such as `/`, `/data`, `C:\` or `C:`; without it, the system disk | the disk is found | | `usage` %, `used` GB, `free` GB, `total` GB |
| `disks` | `ignore` (optional): mount points to leave out ([below](#every-disk)) | a fixed disk is found | | `usage` % and `free` GB of the **fullest** disk, `volumes` (how many) |
| `smart` | `ignore` (optional): drives to leave out, by model or serial number ([below](#disk-health)) | no drive says it is failing (and at least one reports its health) | | `failing`, `warnings`, `drives` (how many), `temperature` °C of the hottest, `wear` % of the most worn, `bad_sectors` |
| `temperature` | `sensors` (optional): the sensors that count, by name ([below](#temperature)) | the machine shows a sensor, and none is at its own critical limit | | `temperature` °C of the hottest, `sensors` (how many count) |
| `hyperv` | `vms` (optional): the virtual machines that must run, by name ([below](#hyper-v-virtual-machines)) | every named machine runs, and none that counts is in a critical state | | `running`, `vms` (how many count), `warnings` |
| `file` | `path`: a file or a folder, as a full path; optional: `pattern`, `recursive`, `min_bytes` ([below](#files-and-folders)) | a file is there (in a folder: one that matches) and its newest isn't smaller than `min_bytes` | | `age` h, `size` MB, `files` |
| `service` | `name`: a Windows service's name, or a systemd unit ([below](#services)) | the service is running (Windows), the unit is active (systemd) | | |
| `process` | `name`: a program's name; optional: `command` (text its command line contains), `min` (default 1), `max` ([below](#processes)) | between `min` and `max` processes of that name run | | `processes`, `cpu` %, `memory` MB |
| `interface` | `name`: a network interface of the probe's machine, as the system names it ([below](#network-interfaces)) | its link is up | | `received` and `sent` Mbit/s, `errors`, `speed` Mbit/s |
| `eventlog` | `log`: a Windows event log; optional: `source`, `ids`, `level` (`critical`, `error`, `warning`, `information`; default `error`), `minutes` (default 60), `count` (default 1), `wanted`, `warning` ([below](#windows-event-log)) | the log can be read (Windows only) | by its own count: Down, or Warning with `warning` | `events` |
| `updates` | optional, each in days or `null` (never): `security_days` (default 0), `reboot_days` (default 0), `stale_days` (default 14) ([below](#pending-updates)) | the machine's system says which updates wait | by its own three limits: Warning, never Down | `updates`, `security`, `security_days` d, `reboot` (0 or 1), `reboot_days` d, `looked_days` d |
| `uptime` | none | always | | `uptime` s |

Examples:

```json
{ "kind": "ping", "host": "192.168.1.1" }
{ "kind": "ping", "host": "203.0.113.10", "count": 10, "tcp_port": 443 }
{ "kind": "http", "url": "https://example.com/health" }
{ "kind": "http", "url": "https://intranet.example.lan/login", "status": "200, 401", "contains": "Sign in", "verify": false }
{ "kind": "tcp", "host": "db.example.lan", "port": 5432 }
{ "kind": "tls", "host": "www.example.com", "port": 443 }
{ "kind": "tls", "host": "nas.example.lan", "port": 5001, "verify": false }
{ "kind": "dns", "name": "intranet.example.com", "server": "192.168.1.1", "expect": ["192.168.1.20"] }
{ "kind": "snmp", "host": "192.168.1.5", "oids": [{ "oid": "1.3.6.1.2.1.1.3.0", "name": "Uptime" }] }
{ "kind": "snmp", "host": "switch.lan", "version": "v3", "user": "monitor", "auth": "sha256", "privacy": "aes128",
  "oids": [{ "oid": "1.3.6.1.2.1.2.2.1.8.1", "name": "Uplink", "expect": ["1"] }] }
{ "kind": "snmp", "host": "printer.lan", "oids": [{ "oid": "1.3.6.1.2.1.43.11.1.1.9", "name": "Supply", "walk": true,
  "rows": "1.3.6.1.2.1.43.11.1.1.6", "of": "1.3.6.1.2.1.43.11.1.1.8", "warn": { "under": 15 }, "down": { "under": 3 } }] }
{ "kind": "database", "engine": "postgres", "host": "db.example.lan" }
{ "kind": "database", "engine": "mssql", "host": "10.0.0.7", "port": 14330 }
{ "kind": "process", "name": "sqlservr" }
{ "kind": "process", "name": "java", "command": "billing.jar", "min": 2, "max": 4 }
{ "kind": "interface", "name": "Ethernet" }
{ "kind": "smart" }
{ "kind": "smart", "ignore": ["WDC WD40*", "S3Z9NB0K"] }
{ "kind": "hyperv" }
{ "kind": "hyperv", "vms": ["web01", "sql-*"] }
{ "kind": "mail", "protocol": "smtp", "host": "mail.example.com" }
{ "kind": "mail", "protocol": "smtp", "host": "mail.example.com", "port": 587 }
{ "kind": "mail", "protocol": "imap", "host": "mail.example.com", "security": "tls", "verify": false }
{ "kind": "ntp", "host": "pool.ntp.org" }
{ "kind": "nut", "host": "nas.lan" }
{ "kind": "nut", "host": "nas.lan", "ups": "rack" }
{ "kind": "temperature" }
{ "kind": "temperature", "sensors": ["nvme*", "drivetemp*"] }
{ "kind": "eventlog", "log": "System", "source": "disk", "ids": [7, 11, 51, 153], "level": "warning", "minutes": 1440, "warning": true }
{ "kind": "eventlog", "log": "Microsoft-Windows-Backup", "ids": [4], "level": "information", "minutes": 1560, "wanted": true }
{ "kind": "updates" }
{ "kind": "updates", "security_days": 7, "reboot_days": null, "stale_days": 30 }
{ "kind": "disk", "path": "/data" }
{ "kind": "disks", "ignore": ["E:", "/mnt/scratch"] }
{ "kind": "file", "path": "D:\\Backups\\SQL", "pattern": "*.bak", "min_bytes": 524288000 }
{ "kind": "service", "name": "Spooler" }
{ "kind": "service", "name": "postgresql@16-main" }
{ "kind": "cpu" }
```

`cpu`, `memory`, `disk`, `disks`, `file`, `service` and `uptime` look at the machine the probe runs on. `ping` needs ICMP sockets: root or `CAP_NET_RAW` on Linux (the systemd unit grants it), administrator rights not needed on Windows. A probe that isn't allowed them still runs; its `ping` checks say so, or use their `tcp_port`.

Thresholds apply to the `usage` metric of `disk`, `disks`, `cpu` and `memory` checks, to the latency of `ping`, `http`, `tcp`, `dns`, `database` and `snmp` checks, to the `loss` of `ping` checks, to the `age` of `file` checks, and to the `days_left` of `tls` checks. The probe only says whether a check worked; the hub decides Warning and Down ([alerting](../guide/alerting.md#statuses)). An `updates` check has no such thresholds: its three limits are part of the check ([below](#pending-updates)). An `eventlog` check is judged by its own count ([below](#windows-event-log)).

### SNMP

An `snmp` check (M5-08, D-86; tables, limits and device templates: M5-09, D-87) asks an SNMP agent for values: a switch's port, a printer's toner, a NAS's volume state, a UPS's battery. One probe on the network asks every device there; nothing is installed on them. It only reads (`GET`, and `GETBULK` for tables): the probe never sets anything.

**For a printer, a switch, a Synology or QNAP NAS or an APC UPS, start from its [template](#templates):** it asks for the device's address and community and adds a check with the values that matter and their limits. The rest of this section is for changing such a check, and for other devices.

**Values** are named by their **OID in numbers** (`1.3.6.1.2.1.1.3.0`; the device's manual or `snmpwalk` lists them; MIB names such as `sysUpTime.0` aren't understood). Each can have a `name` for results, and an `expect`: what it must be, as text (`1` for an interface that is up; several accepted values are separated by `|`). In the form, one a line:

```
1.3.6.1.2.1.1.3.0
Uplink = 1.3.6.1.2.1.2.2.1.8.1
Uplink = 1.3.6.1.2.1.2.2.1.8.1 = 1
```

The check is **Up** when the agent answers with every value and each is what its `expect` says. **Down:** no answer, a value the agent doesn't have, or one that isn't what is expected (`Uplink is 2 (expected 1)`). Numbers (integers, counters, gauges, and time ticks as seconds) become **metrics** with a history; text is shown in the message. Its **latency** is how long the agent took, with the same limits as a ping's.

| Version | Opens the agent | In the form |
|---|---|---|
| `v2c` | a **community** (a password sent in the clear on the probe's network) | Community |
| `v3` | a **user**, optionally authenticated (`auth`: `md5`, `sha1`, `sha224`, `sha256`, `sha384`, `sha512`) and the exchange encrypted (`privacy`: `des`, `aes128`, `aes192`, `aes256`; needs `auth`) | User, Authentication and its password, Privacy and its password (8 characters or more each) |

An agent **doesn't answer** a wrong community or a probe it doesn't allow: the check then says `No answer from 192.168.1.5:161 (SNMP v2c) within 3 s: is SNMP on, is this probe allowed, and is the community right?`. Most devices list the addresses allowed to ask: add the probe's. The probe asks twice before giving up, 3 seconds each.

**How a value is read** is said by options after it, each behind a `;` (in the API: fields of the value):

| Option | API | Meaning |
|---|---|---|
| `walk` | `"walk": true` | The OID is a **column of a table**: every row of it is read (every port, every disk), without naming each. A row is called by its index (`Level: 1.2`) |
| `rows OID` | `"rows"` | A table whose rows are **named by another column** (`ifName`, the disk's name): `Supply: Black Toner` |
| `of OID` | `"of"` | The value is a **share of that one** (its capacity; in a table, the column of it), in percent. Where the device says it doesn't know (a level or a capacity below zero), the value is `unknown` and isn't judged |
| `per second` | `"rate": true` | A **counter** (bytes, errors): how fast it grows. Known from the second reading on (`measuring` until then, and after the device restarts) |
| `x 0.1`, `/ 60` | `"scale"` | The number is multiplied or divided: tenths of a degree, seconds to minutes |
| `unit °C` | `"unit"` | Its unit, at most 12 characters |
| `means 1=up, 2=down` | `"names": {"1": "up"}` | What its values **mean**: results show the meaning, and `expect` and `not` may use it. Such a value is a state, not a measure: no history is kept of it |
| `not down \| failed` | `"not": ["down"]` | What it **must not be** (instead of what it must be: one or the other) |
| `warn < 20`, `down < 5` | `"warn": {"under": 20}` | **Limits:** past `warn` the check is Warning, past `down` Down. `<` under, `>` over (`{"over": 60}`); both on the same side. The hub judges them, like every threshold |

```
Supply = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 15 ; down < 3
Port = 1.3.6.1.2.1.2.2.1.8 ; rows 1.3.6.1.2.1.31.1.1.1.1 ; means 1=up, 2=down
Uplink in = 1.3.6.1.2.1.31.1.1.1.6.24 ; per second ; x 0.000008 ; unit Mbit/s
Temperature = 1.3.6.1.4.1.6574.1.2.0 ; unit °C ; warn > 55 ; down > 65
RAID = 1.3.6.1.4.1.6574.3.1.1.3 ; walk ; means 1=normal, 11=degraded, 12=crashed ; not degraded | crashed
```

`expect` and `not` compare with the value as the agent gives it or with its meaning, whatever the case of the letters; of a table, every row must pass (`Port: ge-0/0/3 is down`). A limit that is reached shows as the check's reason: `Supply: Cyan Toner 9 % is under the warning limit (15 %)`, with how many more values are past theirs. The message lists a table's rows where they are few, and sums them up where they are many (`Port: 18 up, 6 down`; `Disk temperature: 8 rows, 31 to 44 °C`).

**Limits of a table:** at most 64 rows of a table are read (the message says when a table is longer), and a check keeps at most 100 numbers: the rest are in no history and are not judged by limits. For a switch with many ports, read the columns you need in two checks, or name the ports that matter one by one. Every number is a row in the hub's database each time the probe reports: ask for what you look at.

SNMP v1 and traps are not planned (a probe asks; nothing connects to it).

### Secrets

Some checks need something nobody should read back: an SNMP community, a v3 password (D-86). These are the check's **secrets**, apart from its settings:

- **Write-only.** You type one when you add the check; the hub never shows it again, to anyone. Editing the check, an empty field keeps the saved one (the form says *saved: leave empty to keep it*); typing replaces it. The API answers with the names a check has (`secrets_set`), never the values, and the audit log records the names too.
- **Sealed on the hub.** In the database each is encrypted with the hub's data key (`hub_data.key`, the one that seals alert channels' secrets): a copy of the database alone doesn't give them away. Backups hold both.
- **Sent to the probe that needs it**, with its checks, in the hub's signed answer sealed for that probe. The probe keeps them in `config.json` in its private folder (owner-only), as it must to run the check while the hub is unreachable. Whoever is root or Administrator on the probe's machine can read them there.
- **A check that runs needs them all;** a disabled one may wait. A template that asks for the community (the SNMP ones) gives it to the checks it makes, which run at once; a check from a saved [template](#templates) has none (saved templates keep no secrets) and is added disabled: give it its community, then enable it. **Copy checks from…** takes them along.

### Databases

A `database` check (M5-10, D-88) answers "does the database server take connections?". An open port doesn't say so: a server that is starting, recovering or out of connections still accepts TCP, and so does anything else that listens there. The probe speaks the **start of the server's own protocol** and wants a proper answer to it. **It logs in nowhere:** it needs no account and no password, reads no data and can lock no account out.

| `engine` | Server | Default port | What the probe does | The message when it is up |
|---|---|---|---|---|
| `postgres` | PostgreSQL | 5432 | asks for TLS as clients do (and uses it if the server offers it), sends a startup message, reads the first answer; what `pg_isready` does | `PostgreSQL accepts connections`, `… (over TLS)` |
| `mysql` | MySQL, MariaDB | 3306 | reads the server's greeting, answers with a login without a password, reads the refusal | `MySQL 8.4.2 accepts connections`, `MariaDB 11.4.2 accepts connections` |
| `mssql` | Microsoft SQL Server | 1433 | sends the pre-login every client starts with, reads the answer | `SQL Server 2022 (16.0.4135) accepts connections` |
| `mongodb` | MongoDB (3.6 or newer) | 27017 | sends `hello`, the command every driver starts with | `MongoDB 7.0 accepts connections (primary of rs0)` |

**Down:**

- **The server says it serves nobody:** `PostgreSQL answers, but refuses connections: the database system is starting up` (also shutting down, in recovery, `sorry, too many clients already`); `MySQL or MariaDB answers, but refuses connections: Too many connections (error 1040)`; MongoDB's own reason.
- **Something else answers, or nothing does:** `The port is open, but PostgreSQL didn't answer: …` (another service on that port, a proxy, a server that hung up), or `… within 5 s` (it accepted the connection and said nothing: often a server that hangs).
- **The port is closed:** `Connection failed: …`, `Connection timeout`.

**Up, with a note,** when the server works but turns the probe away for who or where it is: `PostgreSQL accepts connections (it turns this probe away: no pg_hba.conf entry for host "10.0.0.9", …)`, or MySQL's `Host '…' is not allowed to connect`. Other clients are served; whether your application's account works is for the application's own check (its web page, its port).

Its **latency** is the time to that answer, with the same limits as a TCP port's.

What to know:

- **The login the probe doesn't have.** PostgreSQL and MySQL are told a user named `blentinel` connects, without a password; it needn't exist, and nothing is done as it. MySQL needs the attempt: a client that hangs up after the greeting counts as a failed connection, and a server blocks an address that does that often (`max_connect_errors`); a refused login doesn't count. **MariaDB writes each refusal to its log** (`Access denied for user 'blentinel'@'…' (using password: NO)`, a warning, at its default `log_warnings=2`): one line per check. PostgreSQL (with or without TLS), MySQL, SQL Server and MongoDB log nothing at their default settings. A PostgreSQL that lets anyone in from the probe's address without a password (`trust`) logs that the role doesn't exist.
- **SQL Server named instances** (`SERVER\INSTANCE`) listen on a port of their own, often one that changes at each start: give the instance a fixed port in SQL Server Configuration Manager and type it. The probe doesn't ask the SQL Browser service.
- **MongoDB that takes only TLS connections** hangs up on the probe's `hello` and shows as down: check its port with a [`tls`](#tls-certificates) check instead. A secondary and an arbiter are up, and the message says what the member is.
- **Not a login test, not a query:** the check says the server takes connections, not that a database in it is healthy. A check that logs in and runs a query is not built.
- **The engines are a list that grows:** Oracle, Redis and others can be added the same way; ask.

### Mail servers

A `mail` check (M5-16c, D-95) answers "is the mail server up, and encrypted as it should be?". The probe connects, waits for the server's greeting in SMTP, IMAP or POP3, starts TLS the way you chose, and says goodbye. **It never logs in and sends no mail**: it needs no account, and leaves one short connection in the server's log.

| Setting | What it is | Usual |
|---|---|---|
| `protocol` | `smtp` (sending, and receiving from other servers), `imap` or `pop3` (mailboxes) | needed |
| `host` | The server: a name or an IP address. With `verify`, the name its certificate is for | needed |
| `security` | `starttls`: the usual port, where the server is asked to start TLS. `tls`: TLS from the first byte. `none`: no encryption, only the greeting | `starttls` |
| `port` | The port | 25, 143, 110 (`starttls`, `none`); 465, 993, 995 (`tls`). Mail programs send on **587** (`starttls`): type it |
| `verify` | `false`: accept a certificate that is self-signed, from a private authority or for another name; only its dates count | `true` |

| State | When |
|---|---|
| **Down** | nothing listens, or what answers isn't that protocol; the server refuses (`421`, `* BYE`, `-ERR`: too many connections, shutting down) |
| **Down** | `starttls` and the server doesn't offer it, or TLS doesn't start |
| **Down** | its certificate has expired, or (with `verify`) a client wouldn't accept it |
| **Warning**, **Down** | its certificate has 21, 7 days left: the [certificate limits](../guide/alerting.md#thresholds) of the site, company or hub, as for a `tls` check. Or the latency is at a limit you set |
| **Up** | otherwise |

**Messages:** `SMTP ready (STARTTLS): mail.example.com ESMTP Postfix. Certificate expires in 59 days (2026-12-11), issued by R11`; `IMAP ready (TLS): Dovecot ready. Certificate expires in …`; `POP3 ready: Dovecot ready.` (not encrypted); `SMTP answers, but it doesn't offer STARTTLS`; `SMTP answers, but its certificate isn't good: Certificate expired 3 days ago (2026-10-10)`; `IMAP answers, but refuses: Too many connections`; `The port is open, but SMTP didn't answer: its answer isn't SMTP's`.

What to know:

- **Which port.** Port 25 is where other mail servers deliver; many networks block it outbound, so a probe at home or in an office often can't reach port 25 of a server elsewhere. For "can our people send and read mail", check **587** (`smtp`, `starttls`) and **993** (`imap`, `tls`).
- **This is the check for a certificate behind STARTTLS.** A [`tls`](#tls-certificates) check makes a TLS handshake at once, which works on 465, 993 and 995 but not on 25, 587, 143 or 110. The `mail` check reads the certificate after `STARTTLS` and judges it the same way, with the same limits.
- **A server that stops offering STARTTLS is Down.** Mail programs then send passwords and mail unencrypted, or refuse to connect. Choose `none` only for a server that really has no TLS.
- **The certificate limits are not set on the check.** The check's own limits are its latency's; the days left follow the certificate limits of its site, company or the hub.
- **Latency** is the whole exchange: connecting, the greeting and, where encrypted, the TLS handshake.
- **Greetings are shortened** in the message (servers add session numbers), and an IMAP server's list of what it can do is left out.
- **Not done:** logging in; sending a test mail and waiting for it to arrive; what the server offers beyond STARTTLS; blacklists and SPF/DKIM records (a [`dns`](#dns) check can watch a record).

### Time servers

An `ntp` check (M5-16d, D-96) answers two questions with one packet: "does this time server answer?" and "is the clock of the probe's machine right?". The probe asks the server the time (SNTP, over UDP) and compares it with its own machine's clock. **It only measures: it never sets a clock.**

| Setting | What it is | Usual |
|---|---|---|
| `host` | The time server: a name or an IP address. A public one (`pool.ntp.org`, `time.cloudflare.com`), or the network's own (a domain controller, the router) | needed |
| `port` | Its UDP port | 123 |

| State | When |
|---|---|
| **Warning**, **Down** | the clocks are 5 seconds, a minute apart: the [clock offset limits](../guide/alerting.md#thresholds), which you can set on the check, the site, the company or the hub |
| **Down** | the server doesn't answer (asked twice), refuses (`RATE`: asked too often; `DENY`), or says its own clock isn't set |
| **Up** | otherwise |

**Messages:** `This machine's clock is 83 ms behind pool.ntp.org (stratum 2)`; `This machine's clock is 2 h 5 min ahead of dc01.lan (stratum 3)`; `No answer from 10.0.0.1 within 2 s, asked 2 times (UDP port 123)`; `No time from dc01.lan: it says its own clock isn't set (not synchronised)`. When a limit is reached: `clock offset 12.3 s is over the warning limit (5 s, from the built-in default): This machine's clock is 12.3 s behind pool.ntp.org (stratum 2)`.

What to know:

- **Whose clock is wrong?** The check compares two clocks and can't say which one is off. Against a public server, it is this machine's. Against the network's own server, add a second check from the same probe to a public one: if both agree the machine is off, it is; if only the local one disagrees, the server is.
- **Why it matters.** One-time codes (TOTP) fail from about 30 seconds, logins in a Windows domain (Kerberos) at 5 minutes, and certificates look expired or not yet valid on a machine whose date is wrong. Logs from machines whose clocks differ can't be put in order.
- **The built-in limits** suit a machine that keeps its time: synchronised machines stay within a fraction of a second. A Windows machine outside a domain synchronises once a week and may drift a second or two; if the warning is too strict for it, raise it on the check or the site.
- **`stratum`** is how far the server is from a reference clock: 1 is next to one, a domain controller is usually 3 or 4. It is reported, not judged.
- **Latency** is the time the question and answer were on their way. The offset is corrected for it, assuming both ways took as long.
- **UDP port 123 outbound** must be allowed from the probe's machine to the server. A firewall that drops it looks like a server that doesn't answer.
- **Don't ask a public server more than once a minute**: they refuse clients that do (`RATE`). The probe asks once per run of the check, twice if the first question got no answer. Where a name has several addresses (a pool), the second question goes to the next one.
- **Not done:** authenticated time (NTS, symmetric keys); a list of servers in one check; the server's own distance from its source (root dispersion); setting the clock.

### UPS

A `nut` check (M5-16e, D-97) answers "is the power out, and how long will the UPS last?". A UPS hangs on one machine by USB or a serial cable, and that machine's **NUT server** (Network UPS Tools, `upsd`) tells the network about it: a Synology or QNAP NAS, a Linux server, a router. The probe asks that server for the UPS's values. **It reads only:** it logs in nowhere and never tells a UPS to do anything.

| Setting | What it is | Usual |
|---|---|---|
| `host` | The machine the UPS hangs on: a name or an IP address | needed |
| `port` | Its NUT server's TCP port | 3493 |
| `ups` | The UPS's name on that server, one word: `ups` on a Synology, `qnapups` on a QNAP, what `ups.conf` says elsewhere | the only one the server has |

| State | When |
|---|---|
| **Down** | the UPS says its **battery is low**, that it is shutting down what it powers, or that its output is off |
| **Warning** | the UPS is **on battery** (the power is out), its battery must be replaced, it is overloaded, on bypass, or reports an alarm |
| **Down** | the server doesn't answer, has no UPS by that name, has lost contact with the UPS (a cable, its driver), or refuses the probe's address |
| **Up** | otherwise |

**Messages:** `Back-UPS ES 700G: on mains power, battery 100 %, 42 min of runtime, load 23 %`; `Back-UPS ES 700G: on battery, battery 85 %, 31 min of runtime, load 23 %`; `Smart-UPS 1500: on battery, battery low, battery 9 %, 15 min of runtime, load 41 %`; `No word of the UPS: the server has 2 UPSes (rack, desk): name one in the check`; `No word of the UPS: the UPS's driver isn't running on the server`.

What to know:

- **The server must allow the probe's machine.** A NUT server answers only the addresses it is told to. Synology: Control Panel, Hardware & Power, UPS, "Enable network UPS server", and add the probe's IP address under "Permitted DiskStation devices". QNAP: External Device, UPS, "Enable network UPS master", and add the address. Linux: `LISTEN` in `upsd.conf` (it listens on the machine itself only, unless told), and the firewall. Reading needs no user or password.
- **The UPS decides when its battery is low**, by the limit set in the UPS or its driver; the check has no limits of its own. Charge, runtime and load are shown and kept in the history.
- **A power cut is a Warning, a battery running out is Down.** With alerts on both, the first says "the power is out at this site" and the second "it is about to go dark".
- **The probe's own machine may be on that UPS.** When the power stays out, the probe goes down with it, and the hub says the probe has stopped reporting: that is the last word.
- **A UPS with a network card** needs no NUT: use its SNMP template ([APC UPS](#templates)).
- **Not done:** limits on charge or runtime; several UPSes in one check; logging in; commands to the UPS (a test, a shutdown); encrypted connections to the server (`STARTTLS`); `apcupsd`'s own protocol.

### Processes

A `process` check (M5-13, D-90) answers "is this program running, how many of it, and what does it use?". It is for what a [`service`](#services) check can't say: a program started by a scheduled task or by hand, a pool of workers that must number at least so many, one `java` among several, a program that must **not** run. The probe only looks: it never starts or ends a process.

| Setting | What it is | Usual |
|---|---|---|
| `name` | The program's name as the task list shows it: `sqlservr`, `nginx`, `backup-*`. `.exe` or not and whatever the case; `*` is any characters, `?` any one. A name, not a path | required |
| `command` | Text the command line must contain, whatever the case: `billing.jar`, `--config /etc/agent.toml`. For programs that share a name | empty: all of that name |
| `min` | The fewest that must run. `0`: none need to | `1` |
| `max` | The most that may run. `0` with `min` `0`: it must not run | none: any number |

**Messages:** `nginx: 3 running, 20% CPU, 100 MB`; `No process named backup-agent is running`; `nginx: 3 running, fewer than the 5 expected`; `java (billing.jar): 6 running, more than the 4 allowed`; `teamviewer: none running`; `teamviewer: 1 running, and none should be`.

What to know:

- **The numbers are of all of them together.** `cpu` is a share of the whole machine since the probe's last cycle (one process busy on one of four processors is 25 %), so the first run after the probe starts says 0. `memory` is what they hold in RAM (the working set on Windows, the resident size on Linux).
- **No thresholds.** Enough run or they don't; CPU and memory are shown and kept in the history, not judged.
- **Linux keeps 15 characters of a program's name.** A longer name is still found: by its program file, by the first word of its command line, or by what Linux kept (`systemd-journald` finds `systemd-journal`).
- **Whose processes the probe sees.** On Windows the probe is a service of the system and sees every process and its command line. On Linux it runs as its own user without rights: it sees every process's name and command line as anyone on the machine does (`ps` shows the same), unless the machine hides other users' processes (`/proc` mounted with `hidepid`), where it sees only its own.
- **A script is a process of its interpreter:** `python3 /opt/app/run.py` is named `python3`. Name the interpreter and put the script in `command`.
- **Threads don't count**, only processes.
- **Not done:** thresholds on a process's CPU or memory, the user a process runs as, suggesting processes from the machine.

### Disk health

A `smart` check (M5-16, D-93) answers "is a drive of this machine about to fail?", by what the drives say of themselves (SMART). One check covers every drive of the probe's machine that reports its health. It complements [`disks`](#every-disk), which says how full they are.

| Setting | What it is | Usual |
|---|---|---|
| `ignore` | Drives to leave out, by model or serial number, whatever the case; `*` is any characters: `WDC WD40*`, `S3Z9NB0K`. For a drive you know about and are replacing | none |

| State | When |
|---|---|
| **Down** | a drive says it is failing: its own SMART verdict, an NVMe drive's critical warning (spare space running out, reliability degraded, read-only), or Windows calls it unhealthy |
| **Warning** | a drive shows a sign of trouble: bad sectors, a SMART attribute past its limit, an NVMe drive that says it is too hot, an SSD that has used all of its rated life, or Windows says it needs attention |
| **Up** | every drive that reports is healthy |
| **Down** | no drive reports its health (a virtual machine), or the machine's system can't be read |

**Messages:** `3 disks healthy, the hottest at 41 °C`; `1 disk healthy, at 34 °C, 12 % of its life used`; `ST4000DM004 (serial ZFN0ABCD): 8 bad sectors; 2 other disks healthy`; `ST3000DM007 (serial W1F2XYZ): the drive says it is failing (SMART); 2 other disks healthy`. The serial number is the one printed on the drive's label: it says which one to pull out.

What to know:

- **The probe asks the machine's system, not the drives.** Sending a drive a command takes administrator rights and a tool that isn't on every machine. The system already reads its drives, and tells any program:
  - **Linux:** the disk service **udisks2**, which Ubuntu, Fedora and most desktop and server installations run. Where it isn't installed (a minimal Debian, a container) the check is Down and says so; installing `udisks2` is the administrator's choice, the probe installs nothing.
  - **Windows:** its storage management (what `Get-PhysicalDisk` shows), from Windows 8 and Server 2012 on.
- **Read once in ten minutes,** whatever the check's interval. The systems themselves read a drive no more often, and no drive is woken for it.
- **What is reported differs by system.** Linux: temperature and bad sectors of SATA drives, temperature and wear of NVMe drives. Windows: temperature and wear, for the installed probe (it runs as the system; a probe started by hand as an ordinary user gets the health without the numbers).
- **Behind a RAID controller** the system sees the controller's volume, not the drives: Windows reports the volume's health as the controller gives it; on Linux such a volume usually reports nothing. Use the controller's own monitoring, or SNMP.
- **A virtual machine's disks report nothing**, so the check is Down there: remove it. The host is where the drives are.
- **Temperature and wear are shown and kept in the history, not judged**, apart from a drive that says itself it is too hot or worn out.
- **Not done:** limits on a drive's temperature here (a [`temperature`](#temperature) check has them, where Linux shows the drive as a sensor); the full table of SMART attributes; starting a self-test; drives of other machines.

### Temperature

A `temperature` check (M5-16b, D-94) answers "is this machine too hot?", by its own sensors: the CPU, the board, the drives. One check reads every sensor the machine shows and judges the **hottest**.

| Setting | What it is | Usual |
|---|---|---|
| `sensors` | The sensors that count, by name, whatever the case; `*` is any characters: `coretemp*`, `nvme*`. For limits that suit one part: a drive is too hot long before a CPU is | every sensor |

| State | When |
|---|---|
| **Warning**, **Down** | the hottest sensor is at a [limit](../guide/alerting.md#thresholds): 90 °C is a warning unless you set another; nothing is Down by default |
| **Down** | a sensor is at the **critical limit its own hardware gives** (a CPU's is usually 100 °C), whatever the limits above are |
| **Down** | the machine shows no sensor, or none by the names given |
| **Up** | otherwise |

**Messages:** `coretemp Package id 0 is the hottest of 5 sensors, at 73 °C`; `nvme Composite: 42 °C`; `nvme Composite is at 85 °C, its critical limit is 85 °C`; `No sensor of this machine is named k10temp*. It has: acpitz temp1, coretemp Package id 0, coretemp Core 0`. When a limit is reached, the reason names the sensor: `temperature 91.5 °C is over the warning limit (90 °C, from the built-in default): coretemp Package id 0 is the hottest of 5 sensors, at 92 °C`.

What to know:

- **Add the check, read its message, then narrow it.** The message names the hottest sensor, and a name that matches nothing lists the ones there are. Names are the chip and the sensor: `coretemp Package id 0` (an Intel CPU), `k10temp Tctl` (AMD), `acpitz temp1` (the firmware's), `nvme Composite` (an NVMe drive), `drivetemp temp1` (a SATA drive, where the kernel module is loaded).
- **Two checks for two limits.** One for `coretemp*` with the default, one for `nvme*, drivetemp*` with a warning at 55 °C: a drive at 60 °C is in trouble, a CPU isn't.
- **Linux:** the kernel's sensor list (`/sys/class/hwmon`), which any program may read. What is there depends on the hardware and its kernel modules; the probe loads none.
- **Windows:** only the firmware's thermal zone, which laptops and some small machines have and **most desktops and servers don't**. Windows has no sensor list of its own: the tools that show CPU temperatures there load a driver of their own, which the probe doesn't do. Where Windows shows nothing the check is Down and says so: remove it. A server's temperatures are better read from its management controller (iDRAC, iLO) over [SNMP](#snmp).
- **A virtual machine has no sensors**, so the check is Down there. The host is where the heat is.
- **A sensor that says nothing sensible** (0 °C, a negative number, more than 150 °C) isn't connected and doesn't count.
- **Not done:** fan speeds and voltages; limits per sensor within one check; sensors of other machines (use SNMP).

### Hyper-V virtual machines

A `hyperv` check (M5-16f, D-98) answers "do the virtual machines of this host run?", by what Hyper-V itself says. **The probe runs on the Hyper-V host** (Windows Server, or Windows with Hyper-V turned on) and reads Hyper-V's own list, as `Get-VM` does. It needs no account, no network and nothing set up on the host, and it never starts, stops or changes a machine.

| Setting | What it is | Usual |
|---|---|---|
| `vms` | The machines that must run, by the name Hyper-V Manager shows; `*` stands for any characters (`sql-*`); capitals don't matter | none: every machine is counted, none must run |

| State | When |
|---|---|
| **Down** | a machine the check names is **off, saved, paused, starting or stopping**; or no machine has a name the check gives |
| **Down** | Hyper-V says a machine that counts is in a **critical state** (its disk is full or gone, for one) |
| **Warning** | Hyper-V says a machine that counts has a failure that isn't critical |
| **Down** | the machine isn't a Hyper-V host, or the probe may not read it |
| **Up** | otherwise |

**Messages:** `All 6 virtual machines running`; `4 of 6 virtual machines running (not running: lab1, template)`; `web01 is off; 1 of 2 virtual machines running`; `No virtual machine here is named mail01; 1 virtual machine running`; `sql-prod: Hyper-V says it is in a critical state; 3 of 4 virtual machines running (not running: lab1)`.

What to know:

- **Name the machines that matter.** Without names the check is Up whatever is off (a host has templates and test machines that are meant to be off); it then shows how many run, and still tells of a critical state or a failure. With names, only those machines count.
- **A name nobody has is Down.** A machine that was renamed or removed would otherwise go unwatched without a word.
- **The installed probe can read Hyper-V** (it runs as the system). A probe started by hand as an ordinary user can't: Hyper-V shows such a user an empty list, and the check says so instead of "no machine".
- **This says what the host knows:** that a machine runs, not that its Windows or Linux has started or answers. Check that with a ping, a port or a web page check of the machine itself.
- **Not done:** a machine's CPU, memory or uptime; its replication and checkpoints; the cluster a host is in; a Hyper-V host asked from another machine. Other hypervisors (VMware ESXi, Proxmox, Xen) are pending: they need an account or a setting on the host (ROADMAP M5-16h to M5-16j).

### Network interfaces

An `interface` check (M5-15, D-92) answers "is this network interface up, and what goes through it?" for an interface of the probe's own machine: the server's uplink, the second card to the storage network, a VPN's interface. For the ports of a switch or a router, use the **Switch or router ports** [template](#templates) (SNMP).

| Setting | What it is |
|---|---|
| `name` | The interface's name as its system shows it. Windows: the name in Network Connections (`Ethernet`, `Wi-Fi`, `vEthernet (Default Switch)`), whatever the case. Linux: the name `ip link` shows (`eth0`, `enp3s0`, `br0`), exactly |

**Messages:** `Ethernet: up at 1 Gbit/s, in 2.4 Mbit/s, out 200 kbit/s`; `eth0: up at 1 Gbit/s, in 54 Mbit/s, out 3.1 Mbit/s, 12 errors`; `Wi-Fi: the link is down`; `eth0: the link is down (lowerlayerdown)`; `No network interface named eth1 (this machine has: eth0, lo)`.

What to know:

- **Down means the link is down:** no cable, no Wi-Fi network, a switch port that is off, an interface turned off, or one that isn't there (a USB adapter pulled out, a VPN that isn't connected). It says nothing about whether the network beyond answers: that is what a `ping` of the gateway is for.
- **The numbers are averages since the probe's last report:** `received` and `sent` in megabits a second, `errors` as the packets with an error, in and out, in that time. A probe's first report after it starts has the link only.
- **`speed` is what the link negotiated,** where the system knows it: a cable that falls back from 1 Gbit/s to 100 Mbit/s shows there. Wi-Fi on Linux and most virtual interfaces have none.
- **No thresholds.** The link is up or it isn't; traffic and errors are shown and kept in the history, not judged.
- **Where the probe reads it.** The counters every system keeps for its interfaces; the link from the files of `/sys/class/net` on Linux and from Windows' own list of adapters. Nothing is run, and the probe needs no rights for it.
- **The probe's page lists the machine's interfaces** under Machine: those that are up and have an address. An interface that is down is not listed there, and can be checked all the same.
- **Some interfaces are up but not counted:** Windows' loopback, and some VPN adapters (Tailscale's among them). They show the link without numbers.
- **Not done:** thresholds on traffic or errors (a share of the link's speed, errors a minute), every interface in one check, suggesting interface checks from the machine.

### Windows event log

An `eventlog` check (M5-14, D-91) looks in an event log of the probe's Windows machine for the events of the last minutes that a source, event IDs and a level pick out. Two questions it answers:

- **"Did something go wrong?"** Events are trouble: disk errors, a service that crashed, a backup that failed.
- **"Did it happen?"** With `wanted`, the events must be there: last night's "the backup finished", a scheduled task that ran.

The probe reads with Windows' own query API (no `wevtutil`, no PowerShell) and only reads: it writes no event and clears no log. On Linux the check is Down: `Event log checks work on Windows only`.

| Setting | What it is | Usual |
|---|---|---|
| `log` | The log, as Event Viewer names it: `System`, `Application`, `Security`, or a channel such as `Microsoft-Windows-Backup` or `Microsoft-Windows-TaskScheduler/Operational` | required |
| `source` | Only events from this source (Event Viewer's "Source"): `disk`, `Ntfs`, `Service Control Manager` | any |
| `ids` | Only these event IDs, at most 20 | any |
| `level` | Events of this level and worse: `critical`, `error`, `warning`, `information` (every event) | `error` |
| `minutes` | How far back to look, 1 to 44640 (31 days) | `60` |
| `count` | How many events it takes | `1` |
| `wanted` | The events must be there: fewer than `count` is the trouble | off: `count` events or more are the trouble |
| `warning` | Trouble is a Warning | off: Down |

**Messages:** `No such event in the last 1 hour`; `3 events in the last 1 day; the latest: disk 51 at 2026-10-12 14:02 UTC: An error was detected on device \Device\Harddisk1\DR1 during a paging operation.` The latest event's text is what Event Viewer shows, cut to a line. When the probe can't read: `No event log named … on this machine`, `The probe may not read the … log`.

Some checks people make:

| For | Settings |
|---|---|
| A disk that starts to fail | `System`, source `disk`, IDs `7, 11, 51, 153`, `warning` and worse, 1440 minutes, as a Warning. It is [suggested](#suggested-checks) for every Windows probe |
| File system damage | `System`, source `Ntfs`, IDs `55, 98, 137` |
| A service that ended unexpectedly | `System`, source `Service Control Manager`, IDs `7031, 7034` |
| Windows Server Backup ran last night | `Microsoft-Windows-Backup`, ID `4`, `information`, 1560 minutes (26 hours), `wanted` |
| An unexpected shutdown | `System`, ID `6008` |

What to know:

- **The check stays in trouble as long as the event is within the minutes it looks back,** then recovers by itself: nobody "acknowledges" an event. Choose the minutes for how long you want it shown: an hour for something to act on now, a day for a disk error.
- **Longer than the interval.** A check that runs every 5 minutes and looks back 5 can miss an event between two runs; look back at least twice the interval.
- **Counted up to 1000:** beyond that the message says `1000 events or more`.
- **The Security log** can be read by the installed probe (it runs as the system). A probe started by hand as an ordinary user may not, and says so.
- **A source that was uninstalled** leaves events without words: the message then ends at the time.
- **Names are checked:** a log and a source are letters, digits, spaces and `-_./()$`.
- **Not done:** matching the event's text, events of several logs in one check, the classic "event log is full" condition, a Linux counterpart (the journal).

### Pending updates

An `updates` check (M5-11, D-89) answers "does this machine have security updates waiting, does it need a restart, and is it still looking for updates at all?". **It only reports** (D-59): the probe installs nothing, downloads nothing, and doesn't make the system look for updates; the system's own schedule does that. It runs no tool: it reads what the system already knows.

| | Where the probe reads it |
|---|---|
| **Windows** | The Windows Update Agent, asked what it already knows (no search on the network). Counted: updates that are not installed, not hidden and not optional, so what Windows would install by itself; drivers and previews offered as optional are left out. A security update is one Microsoft rates or files under "Security Updates" (the monthly cumulative updates are). A restart: the two registry keys Windows keeps while it waits for one (Windows Update's and component servicing's). When it last looked: the last successful search |
| **Ubuntu** (and Debian with `update-notifier-common`) | The count Ubuntu works out for its login message (`/var/lib/update-notifier/updates-available`): what `apt upgrade` would install now. A restart: `/run/reboot-required`. When it last looked: when `apt update` last ended well |
| **Other Linux with PackageKit** (Debian, Fedora, RHEL, openSUSE desktops and many servers) | PackageKit's list of updates, over D-Bus; a security update is one it marks so. Nothing is refreshed. On Debian a restart is read as on Ubuntu; elsewhere the system doesn't say, and the check reports no restart either way |
| **Linux with neither** | Down: `Can't tell which updates wait: this system keeps no list of its updates that the probe can read`. Installing `packagekit` (or `update-notifier-common` on Debian) gives it one |

**Messages:** `No updates wait; the system last looked 5 hours ago`; `7 updates wait, 2 of them security updates; a restart has been needed for 3 days; the system last looked 20 days ago`.

**The hub turns it into a Warning** by three limits of the check, each a number of days:

| Limit | Warning when | Usual | The reason shown |
|---|---|---|---|
| `security_days` | security updates have waited at least that long | `0`: as soon as one waits | `2 security updates wait`; with a limit: `2 security updates have waited 9.5 days (limit 7)` |
| `reboot_days` | a restart has been needed at least that long | `0`: as soon as one is | `A restart is needed to finish updates`; `A restart has been needed for 3 days (limit 3)` |
| `stale_days` | the system hasn't looked for updates for at least that long | `14` | `The system last looked for updates 20 days ago (limit 14)` |

In the form an empty field is the usual limit and `never` turns one off (in the JSON: `null`). Updates that aren't security updates only count: they never warn. The check is **never Down for waiting updates**: Down means the probe couldn't read them. Give it limits that fit how you patch: a client patched monthly might get `security_days` 35.

What to know:

- **Read once an hour**, whatever the check's interval: the list is slow to read (seconds on Windows) and changes slowly. Whether a restart is needed is read at every run, so it clears as soon as the machine is back. Right after the probe starts, the first run may say `Reading the system's updates for the first time`.
- **How long security updates have waited.** Windows says when each was released, and the oldest counts. Linux doesn't say: there it is since this probe first saw them waiting, and it starts again when the probe restarts.
- **Ubuntu's file is read in English only.** A system set to another language for its login message is read through PackageKit instead, if it has it.
- **Ubuntu hands some updates out to a part of its machines at a time** (phased updates). Ubuntu's own count leaves out those not yet due on this machine; PackageKit's includes them. So the two can differ, and the probe prefers Ubuntu's.
- **PackageKit is started for the reading** if it isn't running, once an hour, and writes a line to the journal each time. Its first start on a machine can be slow: if it doesn't come up in time the check is Down saying so, and reads again ten minutes later.
- **On Fedora and its relatives** the check counts the updates; the system says neither when it last looked (so that limit judges nothing there) nor whether a restart is needed.
- **Not every machine is yours to patch:** it is a check you add, not something every probe reports. It is suggested for every Windows and Linux machine.
- **Not done:** which updates they are (names, KB numbers), Linux without either source, a restart needed on Fedora and its relatives, macOS.

### Services

A `service` check (M5-06, D-83) answers "is the backup agent, the database, the print spooler running?" on the machine the probe runs on. It only asks: the probe never starts, stops or restarts anything.

**Windows:** `name` is the service's **service name**, not the display name the Services window lists first: `Spooler`, not *Print Spooler*; `MSSQLSERVER`; `MSSQL$SQLEXPRESS` for a named instance. It is under *Properties → Service name*, or in `sc query` / `Get-Service`. The check is Up while the service is **running**; `stopped`, `starting`, `stopping` and `paused` are Down.

**Linux:** `name` is a systemd unit: `nginx` (meaning `nginx.service`), `postgresql@16-main`, or with its suffix for other kinds (`backup.timer`, `data.mount`). The check is Up while the unit is **active** (or reloading); `inactive`, `failed`, `activating` and a masked unit are Down. A job that runs and ends (a `oneshot` service) is inactive most of the time: watch its timer, or the file it writes ([files](#files-and-folders)), not the service.

Messages: `Spooler is running`; `nginx.service is active (running)`; `nginx.service is failed (failed)`; `No service named Print Spooler (use its service name, not its display name)`; `No unit named ngnix.service`.

**A probe installed before 2026-10-12, on Debian or Ubuntu, watches services only, until `probe install` is run on it again.** Its service ran as a throwaway user (systemd's `DynamicUser`), and those systems' message bus (dbus-daemon) turns such users away, so that probe can't ask systemd. It reads the files systemd keeps about its running services instead, which is enough for "running or not": `nginx.service is active`, `nginx.service is not active`, `No unit named ngnix.service`. It can't tell a failed service from a stopped one that way, and it can't read timers, mounts or sockets: such a check is Down and says `Can't tell whether backup.timer is active: …`. Running `probe install` again (from the new probe) gives the service its own user, `blentinel-probe`, which systemd answers in full (D-84). A probe that only updated itself keeps its old service.

A machine without systemd (and other systems) can't be asked: the check is Down and says so. There are no limits to set: a service runs or it doesn't. A service that is restarting for a moment is one result; the check alerts after 2 in a row like any other.

### DNS

A `dns` check (M5-07, D-82) asks a DNS server for a name, from the probe's network: "does `intranet.example.com` resolve here, and to the right address?". It catches a DNS server that stopped answering, a record that was deleted or changed, and a name that someone points elsewhere.

| Setting | Default | Meaning |
|---|---|---|
| `name` | required | The name to look up, as a full name (`intranet.example.com`); the machine's search domains aren't added |
| `record` | `a` | `a` (IPv4 addresses), `aaaa` (IPv6), `cname` (what it is an alias of), `mx` (mail servers), `txt` (SPF and the like), `ns` (a domain's name servers) |
| `server` | the machine's | The DNS server to ask, as an IP address: the site's own server, or `1.1.1.1` to see what the outside world gets. Without it: the servers the probe's machine uses (from DHCP, read each time) |
| `expect` | any answer | The answers that are fine. **Every** answer must be one of them; an answer that isn't listed fails the check. List them all for a name with several addresses. Written as the answers read: `192.168.1.20`; `10 mail.example.com` for MX; the text for TXT. A final dot, letter case and quotes don't matter |

Messages: `A: 192.168.1.20`; `A: 192.168.1.20, 203.0.113.66; not expected: 203.0.113.66 (expected 192.168.1.20)`; `intranet.example.com doesn't exist (NXDOMAIN)`; `example.com has no MX record`; `No answer: …` (the server didn't answer in 3 s, asked twice).

Each run is a real question: the probe doesn't cache answers and doesn't read the machine's hosts file. The answer's time is the check's latency, with the usual latency limits. It asks plain DNS on port 53 (UDP, then TCP for a long answer): no DNS over TLS or HTTPS, no DNSSEC validation.

**A name in a `ping`, `tcp` or `tls` check** is something else: there the probe's machine resolves it the way its programs do (its DNS servers, its hosts file, its cache), each time the check runs. So `ping printer.lan` follows a printer whose address changes; if the name stops resolving, the check is Down (`printer.lan doesn't resolve to an address`).

### Every disk

A `disks` check (M5-05, D-81) watches **every fixed disk** of the probe's machine without naming them: the probe finds them each time it runs, so a disk added next year is watched from its first report. One check per machine replaces a `disk` check per drive letter.

It is one check with one status, the **fullest disk's**: `usage` is that disk's, and the hub's disk thresholds (90 % warning, 97 % down, unless set otherwise) judge it. The message names every disk, fullest first:

`D:\ 92.0% (80.0 GB free), C:\ 40.0% (60.0 GB free)`

and when a limit is reached, the reason carries that list, so the alert says which disk: `disk usage 92% is over the warning limit (90%, from the built-in default): D:\ 92.0% (80.0 GB free), …`.

What counts as a fixed disk: a volume with a size that is **not** removable (USB sticks), read-only (installer images, snap packages), in memory (`tmpfs`), or another machine's (`nfs`, `cifs`). A disk mounted in several places counts once, by its shortest path. `ignore` leaves out more, by mount point or pattern: `"E:"`, `"/mnt/scratch"`, `"/mnt/*"` (`*` any characters; letter case doesn't matter).

Because it has one status, **one alert covers all disks**: while `D:` is over its limit, `C:` filling up changes the message but sends no second alert, and a silence or the limits you set apply to all of them. For a disk that needs its own limits or its own alert (a database volume that must stay under 70 %), add a `disk` check for it, and leave it out of this one if you don't want it counted twice.

### Ping

A `ping` check sends `count` ICMP echoes (4 unless it says otherwise, at most 20), 200 ms apart, each with 1 second to be answered, and reports (M5-04, D-80):

| | |
|---|---|
| latency | the **average** time of the answers; the hub's latency thresholds apply to it |
| `loss` | the percent of echoes that weren't answered. With 4 echoes: 0, 25, 50, 75 or 100 |
| `latency_max` | the slowest answer |
| `jitter` | how much one answer's time differs from the one before, on average (with at least two answers). A stable 40 ms link has little; a congested one or bad Wi-Fi has a lot |

The check is **Up when at least one echo is answered**. Losing some is a **threshold** on `loss`, beside the latency: by default a warning from 50 % (one lost echo of four happens; half of them is a bad link). It is set for the hub, a company or a site ([alerting](../guide/alerting.md#thresholds)); a check's own limits are about its latency. Send more echoes (`count`) for a finer measure: 10 echoes show 10 % steps.

Messages: `4 of 4 answered, 12 ms on average (max 15 ms, jitter 1.2 ms)`; `3 of 4 answered (25% lost), …`; `No answer to 4 pings`.

**`tcp_port`** is for hosts that are there but answer no ping (many firewalls and Windows machines drop them), and for a probe that may not send echoes. When no echo is answered, the probe tries a TCP connection to that port; if it opens, the check is Up: `No answer to 4 pings; TCP port 443 answers`, with the connection's time as its latency and no `loss`. Choose a port the host really listens on (443 for a firewall's admin page, 3389 for a Windows machine with remote desktop, 22 for SSH).

A down host costs the check `count` seconds; all checks of a report run at once, so the report waits for the slowest.

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

## The probe's machine

A probe tells the hub what its machine is made of (M5-12, D-85): the **system** (`Ubuntu 26.04`, `Windows 11 Pro`, its kernel or build, its architecture), its **fixed disks** (as the [`disks`](#every-disk) check counts them), the **services running** on it, its **network interfaces** with their addresses, and its **default gateway**. The probe's page shows it under **Machine**, with when the probe last said so.

It is sent with the first report after the probe starts, then when something changed (the probe looks once an hour), and once a day. It is read natively, like the checks: nothing is run on the machine. A part the probe can't read is empty (no gateway on a machine without a default route; on Linux without systemd, no services). A probe from before this version sends none until it updates: the section says *Not known yet*.

### Suggested checks

From that, the hub proposes checks the probe doesn't have yet, under **Suggested checks** on the probe's page. Nothing is checked until you add it: **Add** takes one, **Add all suggested** takes them all.

| Suggested | When |
|---|---|
| CPU, Memory, Uptime | the probe has no check of that kind |
| Disks (every fixed disk) | the machine has fixed disks and the probe has no `disk` or `disks` check |
| Disk errors (an `eventlog` check of the System log, as a Warning) | the machine runs Windows and the probe has no `eventlog` check |
| Pending updates | the machine runs Windows or Linux and the probe has no `updates` check |
| Gateway (a ping) | the machine has a default gateway the probe doesn't ping yet |
| A `service` check | a service is running that people usually want watched, and the probe doesn't check it |
| A `database` check of the machine itself (`127.0.0.1`, the engine's usual port) | PostgreSQL, MySQL or MariaDB, MongoDB or the default SQL Server instance is among its services, and the probe has no `database` check of that engine. Not for a named SQL Server instance (its port isn't known) |

The services it knows: on Windows SQL Server and its agent (also named instances), MySQL, PostgreSQL, IIS, Active Directory, DNS, DHCP, Netlogon, Kerberos, DFS replication, certificate services, Exchange, Hyper-V, Veeam, Remote Desktop licensing and gateway, WSUS, Windows Server Backup; on Linux nginx, Apache, Caddy, HAProxy, MySQL, MariaDB, PostgreSQL, Redis, MongoDB, Docker, k3s, libvirt, Proxmox, Samba, NFS, BIND, Postfix, Dovecot, CUPS, Veeam and the Blentinel hub. Any other service is one **Add check** away: its name is in the Machine section's list.

A suggestion you don't want stays listed; it costs nothing. One you added, or covered with a check of your own, is no longer proposed.

### Templates

A **template** is a named set of checks added to a probe in one go: **Templates…** on the probe's page. The checks are then that probe's own: edit, disable or delete them as any other, and changing or deleting the template later changes nothing there. A check the probe already has (the same settings, whatever its name) is left alone, so applying a template twice doesn't double anything.

Built in:

| Template | Asks for | Checks |
|---|---|---|
| **Machine baseline** | nothing | CPU, memory, every fixed disk, uptime |
| **Website** | the host name | the page over HTTPS, its certificate (accepted, and not about to expire), its DNS name |
| **Backup freshness** | the backup folder | the newest file there, judged by its age ([files](#files-and-folders)) |
| **Domain controller** | the domain name | the Active Directory, DNS, Netlogon and Kerberos services, the LDAP port, and that the domain's name resolves. For the probe installed on the domain controller |
| **Printer (SNMP)** | the printer's address, its SNMP community | one check: what it is doing, each toner, drum or other supply as a percentage (warning under 15 %, down under 3 %), the paper in each tray, the pages printed. Any printer with the standard Printer-MIB |
| **Switch or router ports (SNMP)** | the address, the community | one check: the state of every port (how many are up), and each port's input and output errors per second (warning over 1). Any device with the standard IF-MIB |
| **Synology NAS (SNMP)** | the address, the community | one check: system, power and fans (normal), temperature (55/65 °C), every disk's state (not crashed, system partition not failed) and temperature (50/60 °C), every volume's RAID state (not degraded, not crashed) and free space (under 15 %/5 %), whether a DSM update waits |
| **QNAP NAS (SNMP)** | the address, the community | one check: system (55/65 °C) and CPU (75/90 °C) temperature, every disk's state, SMART verdict and temperature (50/60 °C), every volume (`Ready`) and its free space (under 15 %/5 %), fan speeds. QNAP's NAS-MIB |
| **APC UPS (SNMP)** | the address, the community | one check: battery state (normal), charge (under 50 %/20 %), runtime in minutes (under 15/5), temperature (40/50 °C), whether it must be replaced, that it isn't on battery, load (over 80 %/95 %), input voltage. APC's PowerNet-MIB (a UPS with a network card) |

The **SNMP templates** are for SNMP v2c: turn SNMP on in the device's own settings, with a read-only community, and allow the probe's address. The community is typed like a password and becomes the check's [secret](#secrets). For v3, apply the template, then edit the check: choose v3 and give its user and passwords. Each template makes **one check with its values as lines** ([SNMP](#snmp)): change a limit, drop a line your device doesn't have (`RAID: the agent has no such table` says which), or add one. The numbers after "warning" and "down" above are where the limits start; they are yours to move. The OIDs are those of the standard MIBs and of the makers' published ones; a model that keeps a value elsewhere needs its line changed.

For **a port that must be up**, add a line to the switch's check with the port's number: `Uplink = 1.3.6.1.2.1.2.2.1.8.24 = 1`; for its traffic, `Uplink in = 1.3.6.1.2.1.31.1.1.1.6.24 ; per second ; x 0.000008 ; unit Mbit/s` (and `…1.10.24` for out).

**Save this probe's checks as a template** (in the same dialog) keeps each check's name and what it checks, as they are at that moment: set one machine up, save it, and apply it to the next ones. It doesn't keep a check's own thresholds, when it alerts, or what it is behind. A saved template asks for nothing: its checks are added as they were saved, without their secrets.

Templates belong to the hub, not to a company. Everyone who manages checks has the built-in ones, for the probes of their companies. **Saved templates are for users who see every company** (an Owner, an Admin, or a Technician not limited to some): only they see, apply, save and delete them, because a saved template shows what the probe it came from checks. A technician limited to some companies doesn't see them.

## How a change reaches the probe

1. You add, change, disable or delete a check, or change the probe's interval. The probe's config gets a new **revision** (a digest of its content).
2. The probe's next report says which revision it runs. It differs, so the hub's answer carries the new config.
3. The probe validates it, saves it to `config.json`, and runs it at once. Its next report carries the new revision and results for the new checks.

So a change takes effect within one report interval. `GET /api/probes/{id}/checks` shows `sync`: `in_sync`, `pending` (on its way), or `rejected` with the probe's reason, e.g. a check kind that this probe version doesn't know. A refused config isn't sent again: the probe keeps running its previous one until the hub has another. Results from a check the probe no longer has, or from another probe's check, are ignored.

More kinds (DNS, services, SNMP, …) are planned: [proposal 0006](../proposals/0006-check-catalogue.md).
