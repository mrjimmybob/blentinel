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
| `tls` | `host`: a name or IP address; `port`: 1–65535 (the form: empty is 443); `verify` (optional, default `true`) | the server's certificate hasn't expired and, with `verify`, a client would accept it ([below](#tls-certificates)) | ✓ | `days_left` d |
| `dns` | `name`; optional: `record` (`a`, `aaaa`, `cname`, `mx`, `txt`, `ns`; default `a`), `server`, `expect` ([below](#dns)) | the name has a record of that type and, with `expect`, every answer is one of those | ✓ (the answer's time) | `answers` |
| `snmp` | `host`; `oids`: 1–20 values, each `oid` and optionally how to read it (`name`, `expect`, `not`, `walk`, `rows`, `of`, `rate`, `scale`, `unit`, `names`, `warn`, `down`); optional: `port` (default 161), `version` (`v2c`, `v3`), and for v3 `user`, `auth`, `privacy` ([below](#snmp)) | the agent answers with every value, each one what `expect` and `not` allow | ✓ (the answer's time; and each value's own `warn` and `down`) | one per number, named by its `name` or OID (a table's rows: `name: row`), at most 100; `answers` |
| `cpu` | none | always (it measures) | | `usage` % |
| `memory` | none | always | | `usage` %, `used` GB, `total` GB |
| `disk` | `path` (optional): a mount point such as `/`, `/data`, `C:\` or `C:`; without it, the system disk | the disk is found | | `usage` %, `used` GB, `free` GB, `total` GB |
| `disks` | `ignore` (optional): mount points to leave out ([below](#every-disk)) | a fixed disk is found | | `usage` % and `free` GB of the **fullest** disk, `volumes` (how many) |
| `file` | `path`: a file or a folder, as a full path; optional: `pattern`, `recursive`, `min_bytes` ([below](#files-and-folders)) | a file is there (in a folder: one that matches) and its newest isn't smaller than `min_bytes` | | `age` h, `size` MB, `files` |
| `service` | `name`: a Windows service's name, or a systemd unit ([below](#services)) | the service is running (Windows), the unit is active (systemd) | | |
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

Thresholds apply to the `usage` metric of `disk`, `disks`, `cpu` and `memory` checks, to the latency of `ping`, `http`, `tcp`, `dns`, `database` and `snmp` checks, to the `loss` of `ping` checks, to the `age` of `file` checks, and to the `days_left` of `tls` checks. The probe only says whether a check worked; the hub decides Warning and Down ([alerting](../guide/alerting.md#statuses)). An `updates` check has no such thresholds: its three limits are part of the check ([below](#pending-updates)).

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
- **PackageKit is started for the reading** if it isn't running, once an hour, and writes a line to the journal each time.
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
