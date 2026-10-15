# Changelog

All notable changes to Blentinel are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Breaking (pre-alpha, D-30)
- **Protocol v10** (M5-18, D-101): the `discover` command, and a report may carry what the probe found on its network. The hub accepts v9 and v10, so probes from M5-12 on keep reporting and update themselves once you approve the new release. **v8 probes are refused** (`unsupported_protocol_version`): install the new probe on them once with Add probe's command.
- **Protocol v9** (M5-12, D-85): a report may carry the inventory of the probe's machine. The hub accepts v8 and v9, so probes from M4-02 on keep reporting and update themselves once you approve the new release. **v7 probes are refused** (`unsupported_protocol_version`): install the new probe on them once with Add probe's command.
- **Protocol v8** (M4-02, D-73): reports carry the probe's health. The hub accepts v7 and v8, so probes from M2-13 on keep reporting and update themselves once you approve the new release. **v6 probes are refused** (`unsupported_protocol_version`): install the new probe on them once with Add probe's command.
- **Alerts are set up in the UI; `[alerts]` is gone from `blentinel_hub.toml`** (M3-03, D-66). Recipients, company overrides and the SMTP server are not imported: an old `[alerts]` section is ignored, and a hub sends nothing until you add a channel and a routing rule on the new **Alerts** page (and, for e-mail, the mail server under its Settings).
- **Protocol v7 and a new certificate pin** (M2-13, D-61). Probes now pin the SHA-256 of the hub certificate's **public key**, not of the whole certificate, and enrollment codes are version 3. The hub accepts v6 and v7 (v5 is refused). Probes enrolled over **HTTPS** can't update themselves to this version (the update rolls back): install them once with Add probe's command. Probes on plain HTTP update themselves. Make new enrollment codes: older ones are refused (`the code is version 2, from an older hub`).
- **Protocol v6** (M2-06, D-56): reports can carry the results of commands. The hub accepts v5 and v6, so probes from M2-15 keep reporting and update themselves to the new probe once you approve its release. **v4 probes (from before M2-15) are refused:** install the new probe on them once with Add probe's command.
- **Probe downloads come from signed releases** (M2-15, D-55). The hub no longer serves the files in `<data folder>/probes/`: Add probe offers a release the hub holds in `<data folder>/releases/<version>/`, signed with a release key. Run `blentinelmake release keygen` once, then build the hub and probes again (they compile its public key in); `blentinelmake hub publish --with-probes` now needs that key, and `hub install` adds the signed release. A hub built without a key takes no release, so Add probe offers no download.
- **Protocol v5** (M2-15): reports carry the probe's version. The hub accepts v4 and v5, so probes from M2-09 keep reporting to the new hub; they can't update themselves, so install the new probe on them once (Add probe's command).
- **Checks are managed on the hub; the probe has no config file** (M2-04, D-49). `blentinel_probe.toml`, its hot reload and `probe --init` are gone, and `[[resources]]` aren't imported. **Protocol v4:** the hub refuses older probes (`unsupported_protocol_version`), so upgrade the hub and every probe together. Migration `0007` drops earlier results, resource alert state and resource silences. After upgrading: add each probe's checks on its page in the hub (see [reference/checks.md](docs/reference/checks.md)), and delete the old `blentinel_probe.toml`. Check kinds are renamed: `local_cpu` → `cpu`, `local_mem` → `memory`, `local_disk` → `disk`, `local_uptime` → `uptime`; `local_data` is gone (add the four checks). `ping` takes an IP address.
- **API changes:** `POST /api/silence` takes `check_id` instead of `resource_key`; silences have `scope_type` `check` and a `company_id`. `GET /api/probe/{id}/devices` answers `check_id`, `kind` (was `resource_type`) and `metrics` (was `metric_value`/`metric_unit`). `POST /api/probes/{id}` requires `interval_secs`.
- **Protocol v3 and new enrollment codes** (M2-03, D-48). Codes now carry the hub's signing key: create new codes after upgrading the hub, and use probes from this version to enroll. The hub accepts protocol versions 2 and 3; v1 reports (without `protocol_version`) are refused.
- **Probes join by enrollment** (M2-01, D-47). The hub's `[[probes]]` whitelist, `/api/handshake`, the probe's compiled-in `hub_cert.pem` (and `probe/build.rs`), and the probe settings `company_id`, `site`, `hub_url` and `hub_public_key` are gone. To upgrade: delete the hub database (migration `0006`), set the hub up again, add the companies and sites under **Companies**, then for each probe **Add probe** and run `probe enroll <code>` on it. `blentinel_probe.toml` keeps only `interval` and `[[resources]]`; the old keys are ignored, so delete them to avoid confusion.
- **The hub decides each probe's company and site;** the `company_id` and `site` in reports are ignored. Reports no longer create companies or sites (D-44's auto-registration): add them in the UI.
- **The web UI moves to HTTPS on port 3443** (M1-05, D-43). HTTPS is on by default, also for configs without `[server.tls]`: the hub creates a self-signed certificate, serves the UI and API on `https://<hub>:3443`, and keeps plain HTTP on `port` (3000) **only for the probe endpoints**, redirecting everything else to HTTPS. Existing probes keep working unchanged. Open port 3443 in the firewall; the browser warns about the self-signed certificate once (compare its fingerprint with the hub log). `https_port` now defaults to 3443, and "HTTPS only on `port`" is now written `https_port = <port>` (it used to be the absence of `https_port`). `enabled = false` still gives plain HTTP for everything.
- **API scripts must send `X-Blentinel: 1` on every POST** (CSRF protection); without it the hub answers `403 csrf`. Probes are unaffected.
- **User accounts replace the admin token** (M1-01, M1-02). `hub_auth.token`, the `auth_token_path` setting and `hub --reset-admin-token` are gone. The database upgrades itself (migration `0002`). After upgrading, start the hub, open the web UI, and create the first Owner with the **setup code** from the hub's log or `<state_dir>/setup.code`. Delete the old `hub_auth.token` file. An old config that still sets `auth_token_path` loads fine; the setting is ignored. Everyone who was logged in must log in again.
- **Delete the hub database before upgrading.** The schema is now managed by versioned migrations (`hub/migrations/sqlite/`). A database created by an earlier hub is refused at startup with instructions. Stop the hub, delete `blentinel.db` (and `-wal`/`-shm`), and start it again. Probe identities and the `[[probes]]` whitelist are unaffected.
- Upgrade the hub before the probes (protocol v2, see below).
- **New hub install layout** from `install_hub_service.sh`: program in `/opt/blentinel/hub`, config in `/etc/blentinel/blentinel_hub.toml`, data in `/var/lib/blentinel`. On Windows: a scheduled task, with the program in `C:\Program Files\Blentinel\Hub` and data in `C:\ProgramData\Blentinel\Hub`. Existing installs from the old scripts didn't work anyway (D-34); reinstall with the new script.

### Fixed
- **Linux: the admin page's Backups section failed to load** when a file was in the way of the backups folder, so it couldn't say why the backup had failed. Found by running the tests on Linux.
- **The company page sometimes had no Silence buttons** for a user allowed to silence: when the devices arrived before the logged-in user had loaded, the device table was drawn as read-only and stayed that way. It now redraws once the user is known. Found by the CI gate's first run on `main`.

### Added (M0-03: the CI gate, D-102, D-103)
- **Every pull request into `main` runs the CI gate** on GitHub Actions: formatting, clippy (server and browser code), every test on Linux and Windows, `cargo deny` (known vulnerabilities, licences, sources) and the browser tests. `main` accepts nothing that failed. See [docs/ci.md](docs/ci.md), which also lists the repository settings that protect `main` and release tags.
- **`blentinelmake ci [step ...]`** runs the same gate locally (`fmt`, `lint`, `test`, `deny`, `browser`); a test keeps it and the workflow in step.
- **The Rust version is pinned** (`rust-toolchain.toml`, 1.98.1, with clippy, rustfmt and the WASM target): every developer and every CI job builds with the same compiler.
- `deny.toml`: only permissive licences, crates from crates.io only, no wildcard versions. The crates are marked `publish = false` (applications, never published).
- The design of the whole pipeline, to staging and production: [proposal 0013](docs/proposals/0013-ci-cd.md) (D-102, which lifts D-24).

### Security (M0-03)
- **rustls 0.23.46** fixes four advisories the new `deny` check found: TLS secrets in `Debug` output (RUSTSEC-2026-0346), data sent after an ECH rejection (-0347), up to 1 GiB allocated by Brotli certificate decompression before authentication (-0348), and an unauthenticated downgrade to classical key exchange (-0349).
- The hub no longer uses the unmaintained `rustls-pemfile` (RUSTSEC-2025-0134): certificates and keys are read with `rustls-pki-types`, and `axum-server` is 0.8.

### Removed (M0-03)
- `.woodpecker.yml`, the old build-only CI.

### Added (M5-18: network discovery, D-101)
- **Discover network**, on a probe's page: the probe looks at its own network once and the hub lists the devices that are there, with their address, name, hardware address and what they listen on. **Add checks** on a device adds a ping and a check per port.
- **Off until allowed.** A sweep touches devices nobody named, and some networks' owners forbid it: an Admin allows it per company, on the company's page (**Network discovery**). Nothing runs on a schedule.
- A sweep pings the addresses of the probe's private networks (at most 1024), reads the hardware addresses the machine learned, tries 22 well-known ports on the devices that are there, and asks the network's DNS for their names. It never looks at a network with public addresses, and the hub connects to nothing.
- API: `POST /api/companies/{id}/discovery`, `GET /api/probes/{id}/network`, and the command kind `discover`. Migration `0026`.
- See [reference/checks.md](docs/reference/checks.md#network-discovery).

### Added (M5-17: script checks, D-100)
- **A new check kind, `script`:** a script of your own, on the probe's machine, for what no other check measures. The probe runs it and believes it, as a Nagios plugin: exit code 0 is Up, 1 a **Warning**, 2 **Down**; its first line is the message, and values after a `|` (`age=3h`) are kept and drawn.
- **Off until you turn it on, on the machine.** The probe runs only files of the folder `scripts` in its own folder, which doesn't exist until its machine's administrator makes it. The hub can't turn it on, and sends only a file's name: no path, no arguments, no command. A hub in the wrong hands can run what you put in that folder and nothing else.
- The probe refuses a link, and a script or folder that others than its owner (Windows: SYSTEM and Administrators) could change. A script is stopped after 20 seconds.
- Linux: any file that says what runs it (`#!/bin/sh`). Windows: `.ps1`, `.cmd`, `.bat`, `.exe`.
- See [reference/checks.md](docs/reference/checks.md#scripts).

### Added (M5-16g: directory server checks, D-99)
- **A new check kind, `ldap`:** is the domain controller answering? And any other LDAP server: Samba, OpenLDAP, FreeIPA. The probe asks the server for its root entry, which every LDAP server tells anyone. It never logs in and reads nothing of the directory: no account, nothing set up on the server.
- **Down** when nothing answers in LDAP, when a domain controller says it isn't synchronised with its domain, or when the server says it can't serve. The message names the server: `LDAP ready: domain controller dc01.corp.example.com of corp.example.com, global catalog`.
- Not encrypted (port 389) unless you choose StartTLS or LDAPS (636). Encrypted, **the certificate is judged** as a `tls` check judges one, with the same limits for its days left: for the domain controller certificate that runs out and stops every program that logs in through LDAPS.
- Its limits are its latency's.
- **Not done:** logging in with an account (pending, ROADMAP M5-16k).
- **Probes must be updated before they get an `ldap` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-16f: Hyper-V virtual machines, D-98)
- **A new check kind, `hyperv`:** do the virtual machines of a Hyper-V host run? The probe runs on the host and reads Hyper-V's own list, as `Get-VM` does: no account, no network, nothing set up on the host, and no machine is ever started, stopped or changed.
- Name the machines that must run (`web01`, `sql-*`): **Down** when one is off, saved or paused, or when no machine has the name. Without names every machine is counted and none must run. Either way, a machine Hyper-V says is in a critical state is **Down**, and one with a lesser failure is a **Warning**.
- How many machines run is reported and kept in the history.
- A probe started by hand as an ordinary user is told it can't read Hyper-V (Hyper-V shows such a user an empty list); the installed probe can.
- **VMware ESXi, Proxmox and Xen are not done:** each needs an account or a setting on the host, and checks that need nothing on the other side come first (ROADMAP M5-16h to M5-16j).
- **Probes must be updated before they get a `hyperv` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-16e: UPS checks, D-97)
- **A new check kind, `nut`:** what a UPS says of itself, asked of the NUT server of the NAS or machine it hangs on (TCP 3493). The probe reads only; it logs in nowhere and sends the UPS no command.
- **Down** when the UPS says its battery is low, that it is shutting down its load or that its output is off; a **Warning** while it runs on battery, and when its battery must be replaced, it is overloaded, on bypass or in alarm.
- Charge, minutes of runtime and load are reported. The check has no limits: the UPS decides when its battery is low.
- **Down** too when the server has no such UPS, has lost contact with it, or refuses the probe's address; the message says which, and lists the names when a server has several UPSes.

### Added (M5-16d: time server checks, D-96)
- **A new check kind, `ntp`:** the probe asks a time server the time (SNTP) and reports how far its machine's clock is from it, which way, and the server's stratum. It never sets a clock.
- **Limits in seconds**, a new threshold metric (`clock_offset`): on the check, its site, its company or the hub. Built in: a warning 5 seconds off, Down a minute off.
- **Down** when the server doesn't answer (it is asked twice), refuses, or says its own clock isn't set.

### Added (M5-16c: mail server checks, D-95)
- **A new check kind, `mail`:** a mail server greeted in SMTP, IMAP or POP3. The probe waits for the greeting, starts TLS (`STARTTLS` on the usual port, TLS from the start, or none) and says goodbye; it never logs in and sends no mail.
- **Down** when the server doesn't answer in its protocol, refuses, doesn't offer the STARTTLS the check asks for, or shows a certificate that has expired or (unless the check says not to verify) isn't trusted.
- **The certificate behind STARTTLS is watched**, which a `tls` check can't reach: its days left are reported and judged by the certificate limits (21 and 7 days built in). The check's own limits are its latency's.
- Ports: 25, 143, 110, or 465, 993, 995 with TLS from the start; any other (587) can be typed.

### Added (M5-16b: temperature checks, D-94)
- **A new check kind, `temperature`:** how hot the probe's machine is, by its own sensors (CPU, board, drives). The hottest sensor is judged, and the message names it.
- **Limits in degrees**, like every threshold: on the check, its site, its company or the hub. Built in: a warning at 90 °C, nothing Down. A sensor at the critical limit its own hardware gives is Down whatever the limits are.
- **`sensors`** narrows a check to the sensors named (`coretemp*`, `nvme*`), so a drive can have a lower limit than a CPU. A name that matches nothing lists the ones there are.
- **Linux:** the kernel's sensor list, read without rights. **Windows:** only the firmware's thermal zone, which most desktops and servers don't have; the check is Down there and says so. A virtual machine has no sensors.
- Thresholds have a new metric, `temperature` (the admin, company and site pages, and the API's `temperature` field).

### Added (M5-16a: disk health checks, D-93)
- **A new check kind, `smart`:** is a drive of the probe's machine about to fail, by what the drives say of themselves (SMART)? One check covers every drive that reports its health. **Down** when a drive says it is failing; a **Warning** for the signs that come first (bad sectors, a SMART attribute past its limit, an SSD that is worn out or too hot). The message names the drive by model and serial number; the hottest drive's temperature and the most worn one's wear are kept in the history.
- The probe asks the machine's own system, not the drives: **udisks2** on Linux, Windows' storage management on Windows. It needs no administrator rights and no tool such as `smartctl`. A Linux machine without udisks2 can't be read, and the check says so; so does a virtual machine, whose disks report nothing.
- Drives can be left out by model or serial number.
- **Probes must be updated before they get a `smart` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-15: network interface checks, D-92)
- **A new check kind, `interface`:** is a network interface of the probe's machine up, and what goes through it? By the name its system shows (`Ethernet`, `Wi-Fi`, `eth0`). **Down when its link is down** or the interface isn't there. Shown and kept in the history: megabits a second in and out since the last report, packets with an error, and the speed the link negotiated. Messages such as `Ethernet: up at 1 Gbit/s, in 2.4 Mbit/s, out 200 kbit/s` and `Wi-Fi: the link is down`.
- The probe reads the counters and the link state its system keeps; it runs nothing and needs no rights. No thresholds: traffic and errors are not judged.
- **Probes must be updated before they get an `interface` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-14: Windows event log checks, D-91)
- **A new check kind, `eventlog`** (Windows probes): the events of a log that a source, event IDs and a level pick out, in the last minutes. Either they are trouble (disk errors, a crashed service) or, with **these events must be there**, their absence is (last night's backup). Down, or a Warning if the check says so. The message has the count and the latest event as Event Viewer words it: `3 events in the last 1 day; the latest: disk 51 at 2026-10-12 14:02 UTC: An error was detected on device …`.
- **Suggested for every Windows probe:** "Disk errors", the events Windows logs when a disk starts to fail, as a Warning.
- The probe reads with Windows' own API and only reads. On Linux the check says it works on Windows only.
- **Probes must be updated before they get an `eventlog` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-13: process checks, D-90)
- **A new check kind, `process`:** is a program running on the probe's machine, how many of it, and what CPU and memory do they use together? By name (`sqlservr`, `backup-*`; `.exe` or not, whatever the case), and by text of the command line for programs that share a name (`java` with `billing.jar`). Down when fewer than `min` (usual: 1) or more than `max` run; `min` 0 and `max` 0 is a program that must not run. Messages such as `nginx: 3 running, 20% CPU, 100 MB` and `No process named backup-agent is running`.
- The probe only looks; it never starts or ends a process. No thresholds: CPU and memory are shown and kept, not judged.
- **Probes must be updated before they get a `process` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-11: pending updates and restart, D-89)
- **A new check kind, `updates`:** which machines have **security updates waiting**, which **need a restart**, and whether the system is **still looking for updates**. The probe reads what the machine's own system already knows and **only reports**: it installs nothing, downloads nothing and runs no tool (D-59). Windows: the Windows Update Agent and the registry. Linux: Ubuntu's own count where it has one, else PackageKit. Messages such as `7 updates wait, 2 of them security updates; a restart has been needed for 3 days; the system last looked 20 days ago`.
- **Three limits on the check, in days,** make a **Warning** of it (never Down): security updates that have waited (usual: at once), a restart that has been needed (usual: at once), a system that hasn't looked for updates (usual: 14 days). In the form an empty field is the usual limit and `never` turns one off.
- **Read once an hour**, whatever the check's interval; the restart is read at every run.
- **Suggested for every Windows and Linux probe** that has no such check.
- **Probes must be updated before they get an `updates` check** (an older probe refuses a config with a kind it doesn't know).
- A Linux machine with neither Ubuntu's `update-notifier` nor PackageKit can't be read, and the check says so.

### Added (M5-10: database checks, D-88)
- **A new check kind, `database`:** does a **PostgreSQL, MySQL or MariaDB, Microsoft SQL Server or MongoDB** server take connections? The probe speaks the start of the server's own protocol and reads its answer: it needs no account, logs in nowhere and reads no data. Up with what answered (`SQL Server 2022 (16.0.4135) accepts connections`, `MongoDB 7.0 accepts connections (primary of rs0)`); Down with the server's own reason when it serves nobody (`the database system is starting up`, `Too many connections`), or when something else, or nothing, answers on the port. The time to the answer has the latency limits of a TCP port.
- **Suggested for a probe whose machine runs one:** a `database` check of the machine itself, on the engine's usual port.
- **Probes must be updated before they get a `database` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-09: SNMP tables, limits on a value, and device templates, D-87)
- **Five SNMP templates** (Templates… on a probe's page): **Printer** (state, every supply in percent, paper, pages), **Switch or router ports** (every port's state, errors per second), **Synology NAS** and **QNAP NAS** (temperatures, disks, volumes, free space, fans), **APC UPS** (battery, runtime, load, on battery). Each asks for the device's address and its SNMP community and adds one check that runs at once, with limits to start from.
- **A value of an `snmp` check can be a whole table** (`walk`, or `rows OID` to name the rows by another column), **a share of another value** in percent (`of OID`: toner left of its capacity), **a rate** of a counter (`per second`), scaled (`x 0.1`, `/ 60`), given a unit, and given meanings (`means 1=up, 2=down`).
- **Limits on a value:** `warn < 15 ; down < 3` (or `>`). The hub judges them: the check turns Warning or Down and says which value (`Supply: Cyan Toner 9 % is under the warning limit (15 %)`).
- **What a value must not be** (`not degraded | crashed`), and several accepted values (`= 3 | 4 | 5`); both compare with the value or its meaning, whatever the case.
- **A template can ask for a secret:** the SNMP ones ask for the community, typed like a password; it becomes the check's secret and is not in the audit log.

### Changed (M5-09)
- **API:** a value's `expect` is now a list (`"expect": ["1"]`, was `"expect": "1"`). An `snmp` check saved with the old form must be saved again (pre-alpha, D-30).
- **Probes must be updated before they get a check that uses a table, a rate or the other new options** (a probe from M5-08 refuses a config whose value has a list of expected values, and would read a table's OID as one value: Down, "no such value").

### Added (M5-08: SNMP checks, and secrets on a check, D-86)
- **A new check kind, `snmp` (SNMP):** ask a switch, a printer, a NAS or a UPS for values by their OIDs, from one probe on its network. v2c with a community, or v3 with a user, authentication (MD5, SHA-1, SHA-2) and privacy (DES, AES-128/192/256). Each value can have a name and what it must be (`Uplink = 1.3.6.1.2.1.2.2.1.8.1 = 1`); numbers become metrics with a history; how fast the agent answers has the latency limits. Read-only: the probe only sends GET.
- **Secrets on a check:** a community or a password is typed once and never shown again, to anyone. Sealed in the hub's database with `hub_data.key`, sent to the probe in the hub's sealed answer, kept by the probe in its private folder. The API and the audit log name the secrets a check has, never their values. A check made from a template has none and waits, disabled.
- The probe's `config.json` is now written readable only by its owner.
- **Probes must be updated before they get an `snmp` check** (an older probe refuses a config with a kind it doesn't know).
- The database upgrades itself (migration `0025`).

### Added (M5-12: the probe's machine, suggested checks and templates, D-85)
- **The probe's page shows its machine:** the system, its fixed disks, the services running on it, its network interfaces and its default gateway, as the probe found them (when it starts, when they change, once a day). Read natively on Linux and Windows; nothing is run on the machine.
- **Suggested checks:** from that, the hub proposes what the probe doesn't check yet: CPU, memory, every disk, uptime, a ping of the gateway, and the running services people usually watch (SQL Server, IIS, Active Directory, nginx, PostgreSQL, Docker and others). **Add** one or **Add all suggested**; nothing is checked until you do.
- **Check templates** (**Templates…** on the probe's page): a set of checks added in one go. Built in: **Machine baseline**, **Website** (page, certificate and DNS of a host name), **Backup freshness** (a folder's newest file) and **Domain controller**. **Save this probe's checks as a template** makes your own, for the next machine like it. A check the probe already has is left alone.
- API: `GET /api/probes/{id}/machine`, `POST /api/probes/{id}/checks/batch` and `…/checks/template`, `GET`/`POST /api/check-templates`, `POST /api/check-templates/{id}/delete` ([api.md](docs/reference/api.md#checks)). Audit: `check.batch`, `check.template`, `template.create`, `template.delete`.
- The database upgrades itself (migration `0024`).

### Added (M5-06: service check, D-83)
- **A new check kind, `service` (Service):** is a service of the probe's machine running? On Windows a service by its service name (`Spooler`, `MSSQLSERVER`), asked of the Service Control Manager; on Linux a systemd unit (`nginx`, `backup.timer`), asked of systemd over D-Bus. The probe only asks: it never starts or stops anything. See [reference/checks.md](docs/reference/checks.md#services).
- **Linux: the installed probe runs as its own user, `blentinel-probe`** (D-84), no longer a throwaway one (`DynamicUser`): the message bus of Debian and Ubuntu refuses throwaway users, so a probe there couldn't ask systemd about a service. `probe install` makes the user (no login, no password) with the system's `useradd`; `probe uninstall` removes it. The service keeps the same restrictions (it writes nowhere but its folder). **Run `probe install` once on Linux probes installed earlier**: until then such a probe on Debian or Ubuntu reads services from the files systemd keeps, which tells running from not, for services only (no timers or mounts).
- **FP-2 reworded** ([PRINCIPLES.md](docs/PRINCIPLES.md)): the probe installs nothing and needs nothing installed; it may use what every system already has. Checks stay native.
- **Probes must be updated before they get a `service` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-07: DNS check, and names in pings, D-82)
- **A new check kind, `dns` (DNS name):** the probe asks a DNS server for a name: the machine's own servers, or one you name (the site's server, or `1.1.1.1` for the outside view). Record types A, AAAA, CNAME, MX, TXT and NS. With **Answers that are fine**, an answer that isn't on the list fails the check: a record that was changed, or a name pointed elsewhere. The answer's time is its latency. See [reference/checks.md](docs/reference/checks.md#dns).
- **A ping takes a host name too,** not only an IP address: the probe's machine resolves it each time, so a device whose address changes (DHCP) is followed. A name that doesn't resolve is Down.
- **Probes must be updated before they get a `dns` check, or a ping by name** (an older probe refuses that config and keeps its previous one).

### Added (M5-05: every disk in one check, D-81)
- **A new check kind, `disks` (Disk usage: every disk):** the probe finds every fixed disk of its machine each time it runs, so one check per machine watches them all, also a disk added later. The fullest disk's usage is judged by the disk thresholds; the message names every disk, and an alert says which one is full. Removable, read-only, in-memory and network volumes don't count; **Disks to leave out** takes mount points and patterns (`E:`, `/mnt/*`). See [reference/checks.md](docs/reference/checks.md#every-disk).
- The existing `disk` check (one disk, with its own limits and alert) stays.
- **Probes must be updated before they get a `disks` check** (an older probe refuses a config with a kind it doesn't know).

### Added (M5-04: ping with loss and jitter, D-80)
- **A ping sends 4 echoes** (1 to 20: **Echoes each time**), not one, and reports the **packet loss**, the average and slowest answer, and the **jitter**. It is up when at least one echo is answered.
- **A threshold on packet loss,** beside the latency: a warning from 50 % by default, set for the hub, a company or a site on their Thresholds forms (`"loss": {"warning": …, "down": …}` in the API). The worse of latency and loss counts.
- **A TCP port to try without an answer** (`tcp_port`): for firewalls and machines that drop pings, the check is up when that port accepts a connection.

### Fixed (M5-04)
- **Reports refused now and then as `bad_signature`, and lost.** The hub checks a report's signature against the report it parsed, written out again, and a metric with many decimals could come back one bit different (the JSON library's default number parsing isn't exact). The probe drops a report refused that way, so its results were missing. Rare with usage percentages, it showed with averages and jitter (about 1 value in 14). Numbers are now read back exactly (`serde_json`'s `float_roundtrip`), on the hub and the probe. Found by the browser tests.
- **A probe that may not open ICMP sockets no longer fails to start** (an unprivileged account, a container without `CAP_NET_RAW`): its ping checks say so, or use their TCP port.
- **Pinging an IPv6 address works:** the probe only had an IPv4 socket.
- Two ping checks of the same host in one report could take each other's answer (both used identifier 0).

### Changed (M5-04)
- An existing ping check now sends 4 echoes 200 ms apart, so an unreachable host takes it about 4 seconds instead of 2 to say so. Its message changes: `4 of 4 answered, 12 ms on average (max 15 ms, jitter 1.2 ms)` (it was `Responded in 12ms`).
- Older probes ignore the new settings and send one echo; they report no loss.

### Added (M5-03: file and folder check, D-79)
- **A new check kind, `file` (File or folder):** "did last night's backup write a file?". The probe looks at one file, or at the newest file of a folder (optionally only names like `*.bak`, optionally in subfolders too), and reports its **age**, its **size** and how many files match. The check is Down when there is no such file, or when the newest is **smaller than a size you set**. It reads names, sizes and dates only. See [reference/checks.md](docs/reference/checks.md#files-and-folders).
- **Thresholds on the age,** in hours, at every level (check, site, company, hub): by default a **warning at 26 hours, down at 50**, which suits a nightly backup. A weekly one sets its own.
- **Probes must be updated before they get a `file` check** (an older probe refuses a config with a kind it doesn't know).

### Changed (M5-02, D-78)
- **The probe no longer asks the machine's certificate store** (FP-2). `http` checks over HTTPS, and a probe whose hub is behind a proxy with a public certificate, now trust Mozilla's list of public authorities compiled into the probe, like `tls` checks. **An `http` check of a site with a certificate from a private authority that was installed on the probe's machine now fails** ("UnknownIssuer"): tick **Accept any certificate** on it. A hub with its own certificate is unaffected (probes pin it).
- A failed `http` check says why: `HTTP 503 Service Unavailable (expected 200-299)` (it was `HTTP Error: 503 …`), and `Connection failed: …` now ends with the cause (refused, timed out, the certificate).

### Added (M5-02: HTTP expectations, D-78)
- **A web page check says what it expects.** Beyond the URL, an `http` check can set: the **status codes that are fine** (`200, 301-308, 401`; default 2xx), **text the page must contain** and **text it must not** (a site that answers 200 with an error page), **GET or HEAD**, whether to **follow redirects**, **headers** to send, and **accept any certificate**. All in the check's dialog; an existing check is unchanged. See [reference/checks.md](docs/reference/checks.md#web-pages).
- "Too slow" stays a threshold (the latency limits of M3-01), not a setting of the check.
- Not yet: basic authentication. Headers are stored and shown as written, so they are not for passwords or tokens.
- **Probes must be updated before a check uses these settings:** an older probe ignores them and judges the page the old way (any 2xx).

### Added (M5-01: TLS certificate check, D-77)
- **A new check kind, `tls` (TLS certificate):** the probe makes a TLS handshake with a host and port (HTTPS, LDAPS, SMTPS, a NAS's or firewall's admin page) and reports the **days until the certificate expires**. The check is Down when the certificate has expired, or when a browser wouldn't accept it (an issuer that isn't a public authority, another name); **Only watch the expiry date** (`"verify": false`) accepts self-signed certificates and private authorities. In the form, an empty port is 443. See [reference/checks.md](docs/reference/checks.md#tls-certificates).
- **Thresholds on the days left,** at every level like the others (check, site, company, hub): by default a **warning with 21 days left, down with 7**. They work the other way round from usage and latency (at or under a limit), so the warning limit is the higher one; the forms and the API (`"tls": {"warning": …, "down": …}`) refuse them inverted.
- The probe trusts Mozilla's list of public authorities, compiled in (`webpki-roots`), never the machine's certificate store (FP-2).
- Fixed: a dialog taller than the window (the check dialog on a small screen) now scrolls, so its **Save** button stays reachable.
- **Probes must be updated before they get a `tls` check:** an older probe refuses a config with a kind it doesn't know and keeps running its previous one (its page says *rejected*, with the reason). Approve the new probe release first.

### Added (M4-03: commands within seconds, D-76)
- **Run checks now reaches the probe within a second or two**, not at its next report. Between reports the probe asks the hub for commands, on a loop of its own (the report interval doesn't change); the hub holds that request open up to 25 s and answers as soon as a command is queued. Still a connection the probe opens (FP-1). If it can't stay open (a proxy that cuts quiet connections), the probe asks every 30 s; commands also still come with every report.
- **The hub sets the pace:** each answer says when to ask again, so the hub can hold, poll or stop probes asking without updating them. It holds at most 10,000 polls at once, one per probe.
- Protocol: `POST /api/probe/commands` (a signed, sealed `CommandPoll`; the answer a signed, sealed `PollAnswer`), probe-facing on both ports. The protocol stays v8: older probes don't poll, and a new probe treats an older hub's 404 as "ask again in an hour".

### Added (M4-04: hub upgrades, D-75)
- **`hub upgrade`** upgrades the installed hub to a newer release, from GitHub (`[upgrades] release_url`) or `--from` a folder: it checks the release's signature against the hub's release keys, tries the new hub on a copy of the database (`hub self-test`), backs up, switches (the old program stays as `hub.prev`), and puts the old program **and database** back if the new hub doesn't keep running. It then adds the release's probes, ready to approve. Linux; Windows hubs are still upgraded with the new `hub.exe install`.
- **Hub Version** on the admin page: **Check daily** (off by default: it contacts GitHub) and **Check now** say when a newer signed release is out, with the command to run. The hub never upgrades itself.
- `blentinelmake release hub` also writes `release.json` (the version), listed in the signed `SHA256SUMS`.
- New config section `[upgrades]` (`release_url`). API: `GET`/`POST /api/admin/hub-upgrade`, `POST /api/admin/hub-upgrade/check`. Audit action `hub.upgrade_check`. Migration `0023`.

### Added (M4-01: managed probe rollout, D-74)
- **Canaries first:** tick **Canary** for some probes under **Probe updates**; an approved release goes to them first, and to every probe once every canary runs it and the wait you set when approving has passed (60 minutes by default). A canary that refuses it or rolls back **pauses** the rollout. **Pause**, **Resume** and **Roll out to all now** on the admin page. With no canaries, a release goes to every probe at once, as before.
- **Updates by company:** each company follows the rollout (the default), takes no updates, or is pinned to one release; and optionally an **update window** (e.g. 02:00 to 05:00) in local time at each probe's site. The hub decides, so nothing is set on the machines.
- **Signed rollbacks:** `blentinelmake release rollback --to <version> --from <version,...>` lets probes on those versions go back to an older release; add its `.tar.gz` on the hub like a release, then approve the older one. Probes still refuse any older version without one. The update offer carries it (`rollback`, ignored by older probes: the protocol stays v8).
- Each probe shows its **update status** on the admin page: up to date, updating, or why it waits.
- API: `POST /api/admin/probe-updates/pause`, `…/resume`, `…/promote`, `…/probes/{id}/canary`, `…/companies/{id}`; `approve` takes `soak_minutes`; `GET /api/admin/probe-updates` answers `rollout` (replacing `approved`, `approved_by`, `approved_at`) and `companies`, and each probe its `canary` and `update_status`; releases have `rollback_from`. A release a company is pinned to can't be removed. Audit actions `rollout.pause`, `rollout.resume`, `rollout.promote`, `probe.canary`, `company.update_policy`. Migration `0022` replaces the approval table (an approved release is forgotten: approve it again).

### Added (M4-02: probe health and the fleet view, D-73)
- **Each report says how the probe itself is doing:** since when it runs, the reports it holds back, its clock against the hub's (from the hub's last answer), and its last problem of the past day (a report it couldn't deliver or the hub refused, a check that couldn't run, keys it couldn't save). Signed with the report, like the rest.
- **The Probes page** (new in the menu): every probe you can see, with its company and site, last report, version, running since, reports held back, clock, whether it runs its checks, and its last problem. Probes that need attention are flagged with the reason (never reported, not reporting, refused its checks, didn't update, reports held back, clock more than a minute off, had a problem); filter by name, company, site or host, or show only those. The probe page gets a **Health** section.
- `GET /api/probes` adds `silent`, `config_revision`, `config_rejected`, `config_sync` and `health` ([reference/api.md](docs/reference/api.md)). Migration `0021` adds the `health` column to `probes`.

### Changed (D-72: alerts for tickets)
- Alerts about an incident have its number in the subject, the same on every one (first, changes, reminders, escalation, recovery): `[BLENTINEL #12][acme][hq] NAS DOWN`, so a PSA or a mail client keeps them in one ticket or thread. Bodies say `Incident: #12 (open)`; webhooks have `incident.ref` and `incident.state` (`open`, `closed`). How to get tickets in a PSA: [guide/alerting.md](docs/guide/alerting.md#tickets-in-your-psa). Direct PSA connectors (M3-10) and SMS (M3-11) are deferred.

### Added (M3-09: daily digest, D-71)
- **A daily digest e-mail** (Alerts page, **Daily digest**): at an hour in a time zone, to the e-mail channels you choose. Open incidents and who is on them, probes not reporting, the last 24 hours (opened, closed, longest first), uptime per company, a link to the hub. Sent every day, also when all is well; once a day, restart-safe. **Send digest now** to try it.
- API: `POST /api/admin/alerts/digest`, `POST /api/admin/alerts/digest/send`; `GET /api/admin/alerts` has `digest` and `digest_last_sent`. Audit action `alerts.digest`. Migration `0020`.

### Added (M3-08: ntfy, D-70)
- **ntfy channels:** alerts as push notifications on phones, through the public ntfy.sh or your own ntfy server (any URL; an optional access token, kept encrypted). Priorities follow the alert (outages urgent, recoveries normal), and a tap opens the incident. **New random topic** makes a topic nobody can guess; the form warns about a short one. How to run your own server: [install/ntfy.md](docs/install/ntfy.md).

### Added (M3-06: reminders and escalation, D-69)
- Each routing rule can **remind** its channels every 15 minutes to a week (`… NAS STILL DOWN`) and **escalate** once to other channels after 5 minutes to a day (`… NAS DOWN (escalated: not acknowledged for 1 h)`) while nobody acknowledges the incident. Acknowledging or recovery stops both; silences and maintenance windows pause them; an outage that started silenced is never reminded. The escalation's channels hear the recovery too.
- Kept with the incident (a restart neither loses nor repeats one) and on its timeline; webhooks get `event` `reminder` or `escalation` and a `kind`.
- API: routing rules take `remind_every_minutes`, `escalate_after_minutes`, `escalate_to`; incidents have `reminders` and `escalated_at`. Migration `0019`.

### Added (M3-07: check dependencies, D-68)
- A check can be **behind** another check of its probe (the gateway's ping; **Behind** in the check's dialog). While that one is Down, the checks behind it are Unknown (`unreachable: Gateway is down`): no alerts, no incidents, no downtime; the gateway's DOWN alert and incident name them (`unreachable behind it: NAS, Printer`). Chains work; loops and other probes' checks are refused. Copying checks keeps the links. Migration `0018`.

### Fixed (M3-07)
- **Disable / Enable** on the probe's page reset the check's "alert after" counts and its own thresholds: it now sends the whole check.

### Added (M3-05: incidents, D-67)
- **Incidents:** each outage of a check, or of a probe that stopped reporting, is one incident from its first alert to its recovery (Warning to Down and back is the same one). **Incidents** in the menu lists open and closed ones; each has a timeline (opened, changed, acknowledged, notes, closed).
- **Acknowledge** ("I'm on it", with a note) and **notes**, for Technicians and up. Clients see their incidents without the notes.
- Alerts link to their incident; RECOVERED says how long the outage lasted; webhooks carry `incident`.
- A silenced outage still opens an incident (marked: nobody was told). Deleting a check closes its incident.
- API: `GET /api/incidents`, `GET /api/incidents/{id}`, `POST /api/incidents/{id}/acknowledge`, `POST /api/incidents/{id}/notes`. Audit actions `incident.acknowledge`, `incident.note`. Migration `0017`.

### Added (M3-03: alert channels and routing, D-66)
- **The Alerts page** (Owners and Admins): **channels** (e-mail, a signed webhook, Microsoft Teams, Slack, Discord, Telegram), **Send test** on each, its last delivery and error, and **routing rules** by alert (Down, Warning, Recovered, probe stopped reporting), company, site and check kind, in order, with "stop here".
- **Signed webhooks:** a JSON body with `X-Blentinel-Timestamp` and `X-Blentinel-Signature` (`sha256=` HMAC-SHA256 of `<timestamp>.<body>`), so a receiver can check it came from the hub.
- **Secrets are encrypted in the database** with a new key, `hub_data.key` (created beside the others, in backups), and write-only in the UI and API.
- The hub's address (**Settings**), so alerts link to the probe's page. The mail server's security is chosen: STARTTLS, TLS or none.
- Failed deliveries are tried 3 times over about 5 minutes.
- API: `GET /api/admin/alerts`, `POST /api/admin/alerts/settings`, `/channels`, `/channels/{id}`, `/channels/{id}/delete`, `/channels/{id}/test`, `/routes`. Audit actions `alerts.settings`, `channel.create`, `channel.update`, `channel.delete`, `alerts.routes`. Migration `0016`.

### Fixed (M3-03)
- E-mail on port 587 used implicit TLS, which STARTTLS servers refuse: the security is now explicit and STARTTLS by default.

### Added (M3-04: maintenance windows and wider silences, D-65)
- **Maintenance windows:** planned work for the whole company, a site, a probe or a check, once, every week (chosen days) or every month (a day, or the Nth/last weekday: Patch Tuesday), in a time zone (a site's, set on the company page). While one is open nothing it covers alerts (probe expiry included), statuses are still shown (marked *maintenance*), and its results don't count for uptime (the chart shades them). Whatever is still wrong when it closes alerts with the next report; a probe still silent alerts once it has closed.
- **Silences of a probe, a site or a whole company**, besides one check, from the new **Maintenance and silences** page of each company.
- Sites have a **time zone** (IANA, default `UTC`).
- API: `GET`/`POST /api/companies/{id}/maintenance`, `POST /api/maintenance/{id}/delete`; site updates take `timezone`; devices have `in_maintenance`, uptime buckets `maintenance_count`. Audit actions `maintenance.create`, `maintenance.delete`. Migration `0015`.

### Breaking (M3-04)
- `POST /api/silence` takes `{"company_id", "scope": {"kind": "check", "check_id"}, "reason", "duration_hours"}` instead of `{"check_id", …}`, and silences are listed with `scope` and a non-null `company_id` instead of `scope_type`/`scope_id`. Silencing a check that doesn't exist answers 400 (was 404).
- A probe that stays expired is looked at on every sweep (so one whose alert waited for a window alerts when it closes); it still alerts once per outage.

### Breaking (M3-01)
- **Thresholds moved from the config file to the web UI** (D-64). `[alerts.thresholds]` and `[alerts.company_overrides."…".thresholds]` are ignored: set them again on the admin page (the hub's default) and the company page. Without that, the built-in defaults apply (warning at CPU 95 %, memory 90 %, disk 90 %; disk down at 97 %).
- API: `/api/probe/{id}/devices` answers `status` as `Up`, `Warning`, `Down` or `Unknown`, and a `reason`; dashboard and company-probe rows have `devices_warning`; `severity` can be `warning`.

### Added (M3-01: four statuses and the threshold hierarchy, D-16, D-64)
- **Up, Warning, Down, Unknown**, judged by the hub when each result arrives and the same everywhere: the dashboard (warning counts, amber), the probe and company pages (with the reason, e.g. `disk usage 93% is over the warning limit (90%, from the company)`), uptime (Warning counts as up) and alerts. An expired probe's checks show Unknown; a stopped probe shows grey (degraded), a warning amber.
- **Thresholds with warning and down limits** for CPU, memory and disk usage and for ping/HTTP/TCP latency, each taken from the check, its site, its company, the hub's default or the built-in default, limit by limit; `off` switches one off. Set on the admin page, the company page (company or site) and in the check dialog, which show what an empty limit inherits.
- **WARNING e-mails**, and RECOVERED when a warning clears. API: `GET`/`POST /api/admin/thresholds`, `/api/companies/{id}/thresholds`, `/api/companies/{id}/sites/{site}/thresholds`; checks gain `limits` and `limits_in_force`. Audit action `thresholds.update`. Migration `0014`.

### Changed (M3-02: flap suppression, D-63)
- **Alerts wait for 2 results in a row** by default, both ways: DOWN after 2 failed results, RECOVERED after 2 good ones, a usage threshold after 2 reports over it. One lost ping or one CPU spike no longer sends DOWN and RECOVERED a minute apart. A real outage is reported one probe interval later than before.
- Each check can set its own numbers (1–10; 1 alerts at once, as before): **Alert after … failed results in a row; recovered after … good ones** in the check's dialog, `fail_after` / `recover_after` in the API. They don't change the probe's config. Copying checks copies them. Migration `0013`.

### Fixed (M3-02)
- A probe's reports are evaluated for alerts one at a time, in the order they arrived. A probe delivering its buffered reports after an outage had them evaluated at the same time.

### Added (M2-14: a certificate from Let's Encrypt, D-62)
- **HTTPS certificate** on the admin page gets the hub's certificate from **Let's Encrypt** (or its staging service, or another ACME authority such as a company's own CA) for the names you give, and **renews it by itself** (hourly look; renewed with a third of its life left). Browsers stop warning. Validation by **TLS-ALPN** (the authority connects to port 443) or **HTTP** (port 80). **Renew now** asks at once; a failure shows the authority's reason, and retries back off from an hour to a day.
- **Probes notice nothing:** the certificate is issued for the hub's own TLS key, so the pin holds through turning it on, every renewal and turning it off (back to a self-signed certificate with the same key). During a key rotation the hub also gets the next certificate, so the switch serves a trusted one.
- The certificate is swapped in place, without a restart. The ACME account's key is `hub_acme_account.json` in the data folder; backups include it.
- API: `GET /api/admin/certificate`, `POST /api/admin/certificate`, `POST /api/admin/certificate/renew`, and `GET /.well-known/acme-challenge/{token}` (also on the plain-HTTP port). Audit actions `certificate.settings`, `certificate.issue`, `certificate.issue_failed`. Migration `0012`.

### Changed (M2-14)
- `hub doctor` warns about a certificate's expiry with 30 days left, or a quarter of its life for short-lived ones (so a Let's Encrypt certificate, renewed with a third left, warns only when renewal is late), and says how to renew it.

### Added (M2-13: rotating the hub's certificate and key, D-61)
- **Hub keys** on the admin page shows what probes pin (the hub's key, the certificate's key and expiry) and rotates the **certificate**, the **hub key** or both: the hub makes the new ones, announces them in its signed answers, shows how many probes hold them and which don't (and why), and **switches by itself once every probe holds them**. **Switch now** switches earlier (probes not ready must enroll again); **Cancel rotation** drops the new ones. A window (14 days by default) flags a rotation still waiting; it is never forced. Earlier rotations are listed.
- The HTTPS certificate is replaced **without a restart**. After a key switch the previous key opens reports probes buffered before they switched, for 7 days. Enrollment codes made before a switch are revoked. Backups include the rotation's files.
- Probes take the next pins, keep both until the switch, then drop the old ones and keep their buffered reports.
- API: `GET /api/admin/hub-keys`, `POST /api/admin/hub-keys/rotation`, `/rotation/switch`, `/rotation/cancel`. Audit actions `rotation.start`, `rotation.switch`, `rotation.cancel`. Migration `0011`.

### Removed (M2-13)
- The "probe too old for commands" handling (D-56): every probe the hub accepts (v6 and later) takes commands.

### Added (M2-12: backups the hub makes by itself, D-60)
- **Daily backups, on by default** (03:00 UTC, the last 7 kept). The admin page's new **Backups** section turns them off or sets the hour and how many to keep, lists the backups (when, why, size) with **Delete**, shows the next run and a failed last one, and has **Back up now**. **Download** asks for your password again (a backup holds the hub's private key), then saves the file. They are the same files as `hub backup`, so `hub restore` takes them.
- **A backup before every database upgrade.** A new hub that finds an older schema backs up first (`…-pre-upgrade.tar.gz`, the newest 3 kept), then migrates. If a migration fails, it puts the database back as it was, keeps the failed one in `<data>/failed-upgrade-<date>/`, and doesn't start, saying why.
- **`[backups] path`** in the config: where backups go (default `backups` in the data folder; `hub backup` uses it too). Put it on another disk so backups outlive the server.
- API: `GET /api/admin/backups`, `POST /api/admin/backups`, `POST /api/admin/backups/schedule`, `POST /api/admin/backups/{name}/delete`, `POST /api/admin/backups/{name}/download` (with the password) and `GET /api/admin/backups/download/{token}`. Audit actions `backup.create`, `backup.delete`, `backup.schedule`, `backup.download`, `backup.download_failed`. Migration `0010`.

### Changed (M2-12)
- `hub backup` streams the database into the archive instead of reading it into memory.

### Added (M2-10: the one-line hub install, D-57)
- **`curl -fsSL https://github.com/mrjimmybob/blentinel/releases/latest/download/hub.sh | sudo sh`** installs the hub on Linux (x86_64, aarch64): it checks the signature of the release's `SHA256SUMS` against your release key and each file's SHA-256, then runs `hub install`, which also adds the probes for Add probe. Nothing is installed if a check fails. `BLENTINEL_VERSION` picks a release, `BLENTINEL_RELEASE_URL` another location; options after `sh -s --` go to `hub install`.
- **`blentinelmake release hub --targets … --probes …`** (on Linux) builds the files of a GitHub release: the hub as a static musl binary, the signed probe release, `hub.sh` with your release keys, and a signed `SHA256SUMS`.

### Changed
- E-mail alerts use rustls instead of OpenSSL (native-tls), so the hub has no OpenSSL dependency and builds as a static binary. Certificates are still checked against the system's trusted authorities.

### Added (M2-06: the command channel, D-56)
- **Run checks now** on a probe's page: the probe gets the request with the answer to its next report (at most one interval), runs its checks at once and reports straight away. The page's new **Commands** section shows each command, who queued it, where it is (waiting for the probe's next report, sent, done, failed, cancelled, expired) and the probe's result (e.g. `ran 3 checks`); a command not sent yet can be cancelled.
- The hub sends a command in every answer until the probe reports its result, so a lost answer doesn't lose it; the probe runs each command once. A command waits 3 report intervals (at least 15 minutes), then expires. Clicking again while one waits doesn't queue another.
- API: `GET /api/probes/{id}/commands`, `POST /api/probes/{id}/commands` (`{"kind": "run_checks"}`), `POST /api/commands/{id}/cancel`. Audit actions `command.queue`, `command.cancel`. Migration `0009`.

### Added (M2-15: probes update themselves from their hub, D-55)
- **Signed probe releases.** `blentinelmake release keygen` creates an Ed25519 release key on the build machine (the private key never goes to the hub); every hub and probe built afterwards trusts its public key. `blentinelmake release build --targets <triples>` builds the probes and signs them: `publish/probe-release-<version>/` (`manifest.json` with each binary's SHA-256, `manifest.json.sig`, the binaries) and a `.tar.gz`. `blentinelmake release sign <folder>` signs binaries built elsewhere.
- **The hub holds releases and offers one.** Add them on the admin page (**Probe updates > Add release**, the `.tar.gz`), with `hub release add <folder | file.tar.gz>`, or by installing a hub published with `--with-probes`; `hub release list` lists them. The hub refuses a release not signed by a key it trusts, with a binary that doesn't match the manifest, or for a protocol it doesn't speak. **No update is offered until an admin approves a release** (Probe updates > **Approve**; **Stop updates** withdraws it). The section lists the releases, how many probes run each, and every probe's version and last failed update. Add probe offers the approved release, else the newest.
- **Probes update themselves.** In its answer to a report, the hub offers the approved release to a probe running another version. The probe checks the release's signature against the keys compiled into it, refuses an older version (no downgrades), downloads its binary from the hub over its pinned connection while monitoring goes on, checks the SHA-256, and runs it with `--self-test` (it must start, read the probe's files and reach the hub). Only then does it switch: the old binary stays as `probe.prev`, and the service starts the new one (systemd: `Restart=always`; Windows: the service's recovery action; run by hand: the probe starts it).
- **Watchdog and rollback.** A new version must deliver a report within 15 minutes (or 3 intervals) and may start at most 3 times before it does; otherwise the probe puts `probe.prev` back and doesn't try that version again. The probe reports a refused or rolled-back update to the hub, which shows it on the admin page and the company's probe list and stops offering that version.
- **`probe updates off`** (the local emergency brake) refuses every update on that machine, and the hub shows why; `probe updates on` undoes it. `probe status` shows whether updates are possible, a version on trial and the last failure.
- API: `GET /api/admin/probe-updates`, `POST /api/admin/probe-updates/approve`, `POST /api/admin/probe-releases` (the `.tar.gz` as the body), `POST /api/admin/probe-releases/{version}/delete`, `GET /download/releases/{version}/{file}` (probes; no login, both ports). Probe records show `update_failed`; downloads show their `version`. Audit actions `release.add`, `release.approve`, `release.remove`. Migration `0008`.
- The Windows services (hub and probe) now also restart when they stop with an error code, not only when they crash.
- Tests: the manifest format, signatures (trusted, unknown key, tampered manifest or binary), version order; the probe's decisions (downgrade, brake, missing platform, giving up after 3 attempts), the watchdog (crash loop, report deadline), switching and rolling back the binary, a broken binary failing its self-test; the hub's release store (add, list, offer, archive with paths refused, a rebuilt version refused); the release API and its permissions, the offer in the answer to a report (approved, already installed, refused, v4 probe); storage conformance for the probe's version and the approval; a browser test of Probe updates. Verified end to end on a systemd host: a probe service updated itself from 0.1.0 to a signed 0.1.1, then was offered a 0.1.2 that crashes at start and went back to 0.1.1, keeping its identity.

### Changed (company page and enrollment codes, D-54)
- **Enrollment codes have no use limit by default**: one code sets up as many probes as needed until it expires (24 hours by default) or is revoked. **Limit** in Add probe (and `max_uses` in the API) still caps it; the API's default changes from 1 to no limit, and `max_uses` is `null` for codes without one. "Codes not used yet" is now **Codes still valid**, with the uses so far.
- **A company's details are shown as saved**, with **Edit** opening the form (Save or Cancel), instead of an always-open form.
- **Add probe is disabled until the company has a site**, with a hint to add one.

### Added (M2-09: one-line probe install, D-53)
- **Add probe gives a one-line command per platform**: it downloads the probe from the hub, checks its SHA-256 (shown on the hub's HTTPS page) and runs `probe install --code …`. Nothing runs if the download or the check fails. Linux uses `curl` and `sha256sum`; Windows, PowerShell as Administrator (5.1 or 7).
- **The hub serves probe binaries**: `GET /download/probe/{target}` (no login, on both ports) and `GET /api/probe-downloads` (the list with sizes and SHA-256). It offers what's in `<data folder>/probes/`; `hub install` copies the `probes/` folder beside it there, and `blentinelmake hub publish --with-probes <triples>` builds it. The answer to creating a code carries `hub_url`.
- **`probe install [--code <code>]`** (root/Administrator): copies itself into its folder (Linux `/var/lib/blentinel-probe`, Windows `C:\Program Files\Blentinel\Probe`), makes it private, enrolls with the code, registers and starts the service. Linux: a systemd unit registered over D-Bus (no `systemctl`), running as a throwaway user (`DynamicUser`) allowed to ping. Windows: the service `BlentinelProbe`, restarting on failure. Running it again upgrades in place, keeping the identity, enrollment, checks and buffer; with a new code it enrolls again.
- **`probe uninstall`** removes the service and the program and keeps the identity; `--delete-data` removes everything.
- **`probe status`**: service, folder, probe ID, hub, company and site, checks, reports waiting.
- **`probe doctor`**: checks the service, the folder (private), identity, enrollment, that the hub answers through its pinned certificate, the clock against the hub's, the checks and the buffer, with a fix for each problem; exits `1` on a failure.
- Probe builds for Linux are static (musl): one binary for any distribution.
- Tests: CLI parsing; the systemd unit; copying the probe into place; doctor's clock check against a test hub; downloads (listing, hashes, unknown targets, no login, the plain HTTP port); the one-line commands; a browser test of Add probe's command against the served file. Verified on a systemd host: the Linux line installs, enrolls and reports (ping as the throwaway user), upgrade, status, doctor, uninstall. The Windows line verified in PowerShell 5.1 and 7 (right hash runs; wrong hash or no hub runs nothing).

### Removed (M2-09)
- The probe's installer scripts (`install_probe_service.sh`, `.ps1`) and systemd unit template: `probe install` replaces them. A probe package is the binary and its checksum. Probes installed by the scripts (in `/opt/blentinel-probe` or by hand): stop and remove that service, then run `probe install --code <new code>`.

### Added (M2-08: the hub manages itself, D-52)
- **`hub install`** (root/Administrator): copies itself into place, creates the config on a first install (listening on all interfaces, ports 3000 and 3443; `--host`, `--port`, `--https-port`), makes the data folder private (Linux `0700` for the service user; **Windows: SYSTEM and Administrators only**), registers and starts the service, and prints the web address and setup code. Running it again upgrades: the program is replaced, the config and data kept. `--no-service` installs the files only.
- **The hub is a real Windows service** (`BlentinelHub`, automatic start, restarts on failure), logging to `hub.log` beside its config. It replaces the scheduled task.
- **`hub uninstall`** removes the service and the program; it **keeps the config and data** unless `--delete-data`.
- **`hub status`**: service, ports answering, web address, pending setup code, database size and schema, probes and users, certificate expiry.
- **`hub doctor`**: checks the config, service, data folder (exists, writable, private), identity key, database (opens, consistent, not from a newer hub), TLS certificate (expired, or within 30 days), ports and disk space, with a fix for each problem; exits `1` on a failure.
- **`hub backup [--out file]`**: a consistent copy of the database made while the hub runs, the identity key, TLS files and config in one `.tar.gz` (owner-only: it holds the private key). **`hub restore <file>`** (hub stopped) puts them where the current config says, moves the replaced files aside, keeps the backed-up config beside the current one, and refuses a backup from a newer hub.
- Without `--config`, these commands (and the hub) use `blentinel_hub.toml` in the working directory, else the installed hub's.
- Tests: CLI parsing; layout, systemd unit and install config; the Windows folder permissions applied and read back; doctor's checks (keys, certificates, ports, databases from a newer hub, disk space); backup → restore into another folder with users, probes and key; restore refusing a running hub, a newer backup or another file. Verified on a real systemd host: install, upgrade, status, doctor, backup, restore and uninstall.

### Removed (M2-08)
- The hub's installer scripts (`install_hub_service.sh`, `.ps1`), its systemd unit template and the sample config in the package: `hub install` replaces them. A hub package is the binary and its checksum.

### Changed (M2-07: a single-binary hub, D-51)
- **The hub is one file:** its web UI (WASM, JS, CSS, favicon) is embedded in the binary, so there's no `pkg/` folder to deploy and the hub runs from any folder. `blentinelmake hub publish` and the hub installers ship only the binary. Browsers revalidate the files by ETag (a `304` until the hub is upgraded).
- Building by hand takes two steps (front end, then server with `BLENTINEL_EMBED_SITE`); see CONTRIBUTING. `cargo leptos watch` still serves the files from disk, with live reload.
- Upgrading an installed hub: run the new installer; a `pkg/` folder left in the program folder by an earlier install is no longer used and can be deleted.
- Tests: the embedded files' lookup, types and caching; a build check that the installers copy no `pkg/`; the browser tests run the hub from a folder with nothing but its config.

### Added (M2-05: the probe page, D-50)
- **A page per probe** (`/probes/<id>`, from the probe's name on its company's pages): its checks with each one's latest result, **Add check** (a form showing the settings of the chosen kind, checked in the browser with the same code as the hub and the probe), **Edit**, **Disable/Enable**, **Delete**, the **report interval**, and a badge saying whether the probe runs the current checks (**In sync**, **Pending**, **Rejected** with the probe's reason). Refreshes every 15 s. Read-only for Viewers and Clients.
- **Copy checks from another probe**, for setting up a machine like one already monitored. API: `POST /api/probes/{id}/checks/copy`; audit action `check.copy`.
- Tests: the copy API and its scoping; the form's field mapping and validation; browser tests of the page (results, sync, add/edit/delete, interval refused with the reason, Viewer read-only).

### Added (M2-04: checks as hub records, D-49)
- **Checks are records on the hub,** each with a stable ID: renaming a check or changing its target keeps its history, alert state and silence (D-15). Kinds: `ping`, `http`, `tcp`, `cpu`, `memory`, `disk` (any mount point, or the system disk), `uptime`, with typed settings validated by one piece of code on the hub and the probe (`common::checks`).
- **The hub sends each probe its checks and report interval** in its signed answer to a report, whenever the probe runs another revision. The probe runs a new config at once, keeps it in `config.json`, and runs it after a restart or while the hub is down. A config it can't use is refused and reported; the probe keeps the previous one. No file is edited on the client's machine.
- **Several metrics per result:** `memory` reports usage %, used and total GB; `disk` also free GB; the first metric is the headline the UI shows. Thresholds apply to `usage`.
- **Each check is limited to 30 s,** so one slow check can't hold up a report.
- **A probe can only report on its own checks:** results for another probe's check (or a deleted one) are ignored.
- API: `GET|POST /api/probes/{id}/checks` (with the probe's sync state: `in_sync`, `pending`, `rejected` and why), `POST /api/checks/{id}`, `POST /api/checks/{id}/delete`, and `interval_secs` on `POST /api/probes/{id}` (5–3600 s). Audit actions `check.create`, `check.update`, `check.delete`; a "Checks" filter on the Audit page. Reference: [docs/reference/checks.md](docs/reference/checks.md).
- Tests: `CheckSpec` validation and config rules (`common`); config revision, sending and refusal rules (hub service); the check API, config push, sync state, refusal and cross-probe results end to end (`hub/tests/checks.rs`); storage contracts for checks, metrics and config state; the probe's config cache and checks; a browser test with the real probe running the checks the hub gave it.

### Added (M2-03: the hub's signed answer, D-48)
- **Every stored report is answered with a `ReportAck`,** signed by the hub and sealed for the probe, and bound to that report. It is the channel that later brings the probe its checks (M2-04) and commands (M2-06).
- **Revocation reaches the probe:** a revoked probe gets a signed answer saying so, stops reporting, clears its buffer, renames `enrollment.json` to `enrollment.revoked.json`, and waits to be enrolled again. A plain `403` (which anyone on the network could send) only drops the report.
- **Clock-skew warning:** the probe warns when its clock is more than a minute off the hub's, before the hub starts refusing its reports.
- The hub's Ed25519 signing key is derived from `hub_identity.key`: nothing new to back up.
- Tests: ack sealing, signature, binding and forward compatibility (`common`); acks for stored and revoked reports (hub API); the probe's handling of trusted, forged, replayed and revoked answers, over real HTTP; a browser test checking that the real probe trusts the real hub's answers.

### Added (M2-01: probe enrollment, D-47)
- **Add probe** on a company's page: choose the site, optionally a name, how many probes may use the code (1) and for how long (24 h), and the hub address probes will use. The hub shows an enrollment code (`BLT1-…`) once. Pending codes are listed and can be revoked.
- **`probe enroll <code>`** joins the hub: the code carries the hub's address, key and certificate fingerprint, so the probe trusts that hub from the first packet and reports over **HTTPS pinned to its certificate**, with the stock binary. Enrolling again moves a probe to another hub, company or site. A probe started before enrolling waits for it.
- **Probe records:** each company page lists its probes (site, state, hostname, version, last seen); rename, move to another site, **Revoke** (`403 probe_revoked`).
- **Add company** (Admins) and **Add site** (also Technicians in their companies).
- API: `POST /api/enroll` (probes), `GET /api/probes`, `POST /api/probes/{id}`, `POST /api/probes/{id}/revoke`, `GET|POST /api/enrollment-codes`, `POST /api/enrollment-codes/{id}/revoke`, `POST /api/companies`, `POST /api/companies/{id}/sites`. Audit actions `probe.enroll`, `probe.update`, `probe.revoke`, `code.create`, `code.revoke`, `company.create`, `site.create` (replacing `company.register`, `site.register`).
- Tests: enrollment API tests (codes expired, used up, revoked, wrong; spoofed company ignored; revoked probes), storage contracts for records and codes, a pinned-TLS test on the probe, and browser tests with a real probe enrolling and reporting over pinned HTTPS.

### Added (M1-06: audit log, D-46)
- **Audit log:** who did what, when and from where: logins and failed logins, logouts, setup, password changes, user management (UI and `hub user`), companies and sites registered by reports or edited, archiving, silences, data deletion, probe removal, archives. Passwords and tokens are never recorded.
- **Audit** page (Owner, Admin): filters by user, action group, target and dates; "Load older"; **Export CSV**. API: `GET /api/audit`, `GET /api/audit.csv`. Guide: `docs/guide/audit.md`.
- The log isn't copied into archives. Writing it is best effort: a failure is logged and never blocks the action.

### Added (M1-04: which companies each user sees, D-45)
- **Company scope per user:** all companies (the default, so a single-admin hub needs nothing), only chosen companies, or all except chosen ones (new companies included). Owners and Admins always see everything. Any number of users can share a company.
- **The Client role** is available: a customer's read-only login for their own companies, without your internal notes and contacts.
- **Technicians can edit the details** (names, contacts, notes, addresses) of the companies in their scope. Archiving a company is now Admin-only.
- **Everything is scoped:** dashboard, companies, probes, devices, uptime, silences, archives. Other companies are left out of lists, and asking for one directly answers `404`, so they can't be discovered.
- **Users page:** a company picker (All / Only these / All except these) when adding or editing a user, and a Companies column. The API's user `all_companies` + `companies` fields are replaced by `scope` (`docs/reference/api.md`).
- Tests: a scoping test for every route open to scoped users (a new data route fails the build until it has one), storage contracts for the three modes, browser tests for a Client and the picker.

### Added (M1-03: companies and sites, D-44)
- **Companies and sites are records.** The first report naming a company or site registers it, marked **New**; nothing to create beforehand. **Companies** in the menu lists them; each company's page edits its display name, contacts and notes, and its sites' names, addresses and notes (Owner, Admin). The dashboard and company pages show the display names. Guide: `docs/guide/companies-sites.md`.
- **Archiving a company** hides it from the dashboard (e.g. a typo in a probe's `company_id`, or a former client); its probes still report and alert, nothing is deleted.
- Existing data is picked up by the migration: every company and site already in the database becomes a record.
- New permission `ManageCompanies` (Owner, Admin); new endpoints `/api/companies…` (`docs/reference/api.md`).
- Known limitation (pre-alpha, D-30): archives created before this version can't be opened in the archive viewer (their database has no `companies` table).

### Added (M1-05: HTTPS by default, D-43)
- **Split mode** (above): passwords and session cookies never cross plain HTTP, probes don't need a rebuild. Session cookies get `Secure` over HTTPS.
- **Login rate limiting:** 5 failed logins per client and username are free, then a lock of 30 s doubling up to 15 min; 20 failures from one client within an hour lock it for 15 min. Wrong setup codes count the same way. Locked attempts get `429 too_many_attempts` with `Retry-After`.
- **CSRF header** on every POST (above), in addition to `SameSite=Strict`.
- **`[server] behind_proxy`** for a hub behind a TLS-terminating reverse proxy (`Secure` cookies, client address from `X-Forwarded-For`).
- Tests: split-mode listeners with a real HTTPS client, CSRF over every POST route, cookies, rate limits; browser tests now run the UI over HTTPS with the probe on HTTP.

### Added (M1-01, M1-02: users and roles)
- **Personal accounts with roles:** Owner, Admin, Technician (views and silences), Viewer (read-only). Log in with username and password; passwords are argon2id-hashed and need at least 12 characters. Guide: `docs/guide/users.md`.
- **First-run setup:** a hub without users creates a one-time setup code (logged, and in `setup.code`), and the web UI's **Set up Blentinel** page creates the first Owner with it.
- **Users page** (Owner, Admin): add users with a temporary password, change role, deactivate, reset password, log a user out. **My account:** change your password, log out your other sessions. The header shows who is logged in, and menus and buttons follow your role.
- **Every API route checks a permission**, declared in one table (`hub/src/routes.rs`); `403 forbidden` otherwise. New endpoints: `/api/auth/login|logout|me`, `/api/setup`, `/api/me/password`, `/api/me/logout-others`, `/api/users…` (`docs/reference/api.md`). `/api/login` and `/api/logout` are replaced.
- **Sessions are stored in the database** (only a hash of the cookie), survive hub restarts, and end after 12 idle hours or 7 days. Deactivating a user or resetting their password ends their sessions. The cookie is now `SameSite=Strict`.
- **`hub user list | create | reset-password`**: account recovery on the server (break-glass), with temporary passwords.
- Archives never contain accounts or sessions.
- Tests: user/session storage contracts, an access test over every route × role, login/session/setup tests, and browser tests for setup, login, a Viewer's restricted view and creating a user.

### Fixed (installers, D-34, M0-22)
- **Upgrading a Linux probe no longer deletes its identity.** The old installer's `rsync --delete` removed `identity.key`, giving the probe a new ID that had to be re-registered. Upgrades now replace only the binary and keep the config.
- The Linux service units start: the probe's `User=` line had an inline comment, and the hub's `--config=…` pointed at a directory in a form the argument parser rejects. The installers now create the `blentinel` service user.
- Ping works without root under systemd (`AmbientCapabilities=CAP_NET_RAW`, which survives upgrades, unlike `setcap`).
- The installers copy only the files a package ships, never runtime files (keys, token, database) left in the package folder.
- The Windows hub installer registers a scheduled task instead of a Windows service the hub can't run as.
- The hub finds its web assets next to its executable, so it can run with any working directory.
- Verified end to end on Ubuntu 24.04 with systemd: install, upgrade (identity, token and config kept; stray runtime files not copied), and ping from the non-root service user.

### Security
- **Private keys and the admin token are now created readable only by their owner** (`0600` on Linux/macOS): `identity.key`, `hub_identity.key`, `hub_auth.token`, and the TLS key. They used to be world-readable (`0644`). The installers' folders already blocked other users, but tighten existing files: `chmod 600 identity.key` (probe), `chmod 600 hub_identity.key hub_auth.token` (hub). On Windows, see `docs/security.md` (D-34 addendum).
- **The hub now rejects unsigned probe reports (403).** Previously the signature was only checked if present, so anyone who could reach the hub could forge reports for any registered probe (a probe's ID and the hub's key are both public). Official probes always sign, so no probe changes are needed (D-25, M0-16).
- **Replay protection.** The hub rejects reports older than the probe's latest stored report (409) and reports outside the accepted time window, more than 5 min in the future or older than 7 days (422). Probe clocks must be within 5 minutes of the hub, so keep NTP running (D-28, M0-17).

### Fixed
- **HTTPS on the hub never started.** The build contains two TLS crypto backends (from different dependencies), so rustls panicked when the hub built its TLS configuration. The hub now picks one (aws-lc-rs). Found by the M0-23 listener tests; now covered by a regression test.
- **Unmuting a device right after muting it didn't unmute it.** The page showed the device as unmuted, but the hub kept the silence, so its alerts stayed suppressed until the silence expired (up to forever). The UI didn't know the new silence's ID and deleted silence 0 instead. It now reloads the silences from the hub after every change (found by the new browser tests, M0-25).
- **The archive viewer showed almost nothing (D-26, M0-19).** Probes that were still reporting disappeared from archives, and the archive uptime chart was always empty (its range ended "now", after all the archived data). Archives now keep every probe they have reports from, the uptime range ends at the archive's newest report, and the `range` parameter is honoured. Archive company cards now open a read-only company page with the probes, devices and uptime chart. A report exactly at the cutoff is no longer copied into the archive as well as kept in the main database. Only new archives benefit; archives made earlier have already lost those probes' records.
- The admin page showed a hand-typed version, build number (`1023`) and build date (`2026-03-08`) that were never updated. It now shows the real version from `Cargo.toml` and no made-up build details (M0-10).
- The dashboard and company pages leaked their 15-second refresh timer's callback on every visit. It is now freed when you leave the page (M0-10).
- The web UI can no longer crash on a missing browser object (no `unwrap()` left in UI code).
- Documentation corrections: payload sealing does **not** give forward secrecy against theft of `hub_identity.key` (earlier docs claimed "perfect forward secrecy"); only resource-level silences exist; the probe's `hub_url` and `hub_public_key` need a restart to change; the hub *can* be cross-compiled.
- `--help` shows the real binary names (`hub`, `probe`), not `blentinel_hub` / `blentinel_probe`.
- A probe that came back online while its "probe offline" e-mail was still being sent could stay marked as offline for alerting, so its next outage wasn't alerted. The alert is now recorded before sending (D-27 addendum).
- README build instructions: `cargo install cargo-leptos` (was `leptos-cli`), the GitHub clone URL (was Codeberg), and the hub *can* be cross-compiled. Build instructions now live only in `CONTRIBUTING.md`.
- **Disk, CPU and memory threshold alerts now fire.** They were only checked when a resource changed between Up and Down, and local metrics are always Up, so a disk at 95 % never alerted. Thresholds now alert once when crossed, stay quiet while exceeded, and re-arm after dropping back (D-33, M0-21).
- **A silence now ends when the silenced resource recovers.** Previously it outlived the outage it was created for. No recovery alert is sent for a silenced outage (D-33, M0-21).
- A probe that went offline a second time no longer stays silent: the "probe offline" alert re-arms whenever the probe reports again (D-27, M0-18).
- **Probe offline buffer no longer grows without limit.** Reports the hub permanently rejects (e.g. while the probe isn't registered yet) are dropped instead of queued forever. Delivered reports are deleted, backlogs are delivered oldest-first, and the buffer is capped at 7 days / 20,000 reports (D-29).
- **Dashboard severity is consistent.** A company with a live outage is now shown red even if another of its probes has expired; previously the card showed amber and hid the outage. Stale "down" counts from an expired probe now make a company *degraded*, not *critical*. The hub computes severity once and returns it as a new `severity` field (`critical` / `degraded` / `healthy`) on dashboard companies and company probes (D-31, M0-08).
- The archive viewer reports probe names the same way as the live API (whitelist name, or empty; the UI shows the short ID).

### Changed (protocol)
- **Probe ↔ hub protocol v2** (M0-06): reports carry a signed `protocol_version`, and responses carry an `X-Blentinel-Protocol` header. The hub still accepts v1 probes unchanged. Upgrade the hub before the probes.
- The hub's `/api/report` status codes are now documented and meaningful to the probe: 200 stored, 409 already have it, 400/401/403/422 permanent rejection, 5xx retry.

### Added
- **Documentation rewrite (M0-12):** a short README, plus `docs/architecture.md`, `docs/security.md`, install guides (`docs/install/hub.md`, `probe.md`, `https.md`), guides (alerting, data retention, troubleshooting) and references (configuration, CLI, HTTP API, probe protocol, database). The content of the old LaTeX manual and the HTTPS/hot-reload notes was moved here and corrected where it had become wrong.
- **Generated PDF manual (M0-15):** `blentinelmake docs` builds `target/manual/blentinel-manual.pdf` from `docs/` with pandoc. The hand-written `manual/manual.tex` is retired (D-09).
- `probe/blentinel_probe.example.toml`: an anonymised example config (documentation IP ranges), kept valid by a unit test that also checks the `probe --init` template. The local `probe/blentinel_probe.toml` is no longer tracked.
- MIT licence (`LICENSE`) for the whole workspace (replaces the Leptos template's Unlicense in `hub/`), and `CONTRIBUTING.md` with build, test and contribution instructions (M0-01).
- Hub integration-test harness (`hub/tests/`): the real API router on a temporary SQLite database with a controllable clock, plus a test probe that seals and signs reports with the production crypto code. Tests cover ingestion, auth, dashboard, uptime, expiry, silences, admin, and alert state. `cargo test-all` runs everything (ROADMAP M0-04).
- **Browser tests for the web UI** (`hub/end2end`, Playwright): a real hub and a registered probe in a temporary folder, driven in Chrome. They cover login, redirects, dashboard, refresh, theme, company drill-down, uptime range, silencing and the admin page. It replaces the Leptos template test. Instructions: CONTRIBUTING.md (M0-25).
- `cargo lint` and `cargo lint-wasm` aliases (clippy with warnings as errors, for the server and the browser code). The workspace is now `cargo fmt`-clean and warning-free on Windows and Linux (M0-24). `.git-blame-ignore-revs` hides the formatting commit from `git blame`.
- `known_issue_*` tests (ignored) that pin confirmed bugs until they are fixed. None are open now.
- Alerting tests (`hub/tests/alerts.rs`) through a `Notifier` interface: SMTP in production, a recording notifier in tests. Alerts were previously untestable.

### Changed
- **Every config setting now reloads without a restart (M0-23, D-38).** On the hub: `host`, `port` and `[server.tls]` (the hub moves its listeners), `state_dir`/`db_path` (switches database; data isn't moved), `identity_key_path` and `auth_token_path`. On the probe: `hub_url` and `hub_public_key`. A reload is all or nothing: if the new file is invalid or something it needs fails (a port in use, a database that can't be opened, a bad hub key), nothing changes and the error is logged. `hub --reset-admin-token` now works on a running hub (the token file is read at each login).
- A probe `hub_url` without `http://` or `https://` is now refused with a clear message.
- **JSON errors everywhere (M0-11, D-35).** Every hub error response is `{"error": {"code", "message"}}`, including `POST /api/report`, instead of plain text or an empty body. Status codes are unchanged. Codes: `reference/api.md`, `reference/protocol.md`. Internal errors say "see the hub log" and no longer include the cause. The admin delete/remove endpoints answer `{"success": true}` instead of `OK`.
- **The probe logs why the hub rejected a report**, e.g. `Hub answered 403 Forbidden: report from unregistered probe a773d201 (unknown_probe)`, instead of just the status.
- **Hub logging uses `tracing`** (timestamp and level on every line) instead of `println!`. Rejected reports, alerts, reloads, archiving and errors are always logged; `--verbose` adds a line per accepted report; `--debug` adds the full report. `RUST_LOG` overrides the flags. Log line texts changed (e.g. `[ALERT ERROR] …` → `ERROR Failed to send … alert`).
- The hub exits with a clear error instead of panicking when its identity key or admin token is unreadable, empty or corrupt. In HTTP+HTTPS mode, a failing HTTP listener now stops the hub instead of dying silently in the background.
- The web UI shows the hub's error message (e.g. why archiving failed), and treats a failed "unmute" as a failure.
- The probe and hub now share key-file handling, the config-file watcher and their common command-line flags (`common::fs`, `common::watch`, `common::cli`) instead of keeping two copies (M0-07, D-37). A probe with an unreadable or corrupt `identity.key` now exits with a clear message instead of panicking. `--config` without a path says `--config requires a value.`
- The web UI code is split into one module per page and per component (`hub/src/app/`), instead of one 1,960-line file (M0-10). No visible change except the fixes below.
- The remaining business rules moved into services (M0-09 complete): `service::alerts` (with a `Notifier` interface and a context struct instead of 11-argument functions), `service::liveness` (probe-expiry sweep, previously in `main.rs`), `service::retention` (storage info and archiving, previously duplicated between the admin API and the background monitor), and `service::silences`. The separate `archive` module is gone. No behaviour change, except `--verbose` expiry messages no longer repeat the timeout.
- Report ingestion is a service (`hub/src/service/ingest.rs`): typed `IngestError`s instead of early HTTP returns, and the authentication steps are a pure function that can be tested without a database. The HTTP handler only maps errors to status codes (same codes as before). `--verbose` now prints one "Report saved" line per accepted report, and `--debug` prints the accepted report rather than every decrypted payload (M0-09).
- The hub reaches its database only through storage traits (`hub/src/storage/`: `ReportStore`, `ProbeStore`, `AlertStore`, `ArchiveStore`, bundled as `Storage`). SQLite is the embedded implementation, and all SQL lives in `storage/sqlite/`. A shared conformance suite (`hub/tests/storage_conformance.rs`) defines the contract for current and future backends. No behaviour change (M0-14, D-32).
- API response types and the rules the UI and server share (severity, resource keys, short probe IDs) are defined once in `hub/src/model.rs` instead of separately in the server and the UI (M0-08).
- All crates share one version and edition (2024) via `[workspace.package]`, and shared dependencies are declared once in `[workspace.dependencies]` (M0-02).
- Database connections now get WAL, foreign keys, and a busy timeout on **every** pooled connection (previously only one), and resource statuses are deleted with their report (`ON DELETE CASCADE`) (M0-05).
- Probe ↔ hub sealing and signatures now live in one place, `common::crypto` (behind the `crypto` feature). The probe and hub no longer carry separate copies. The wire format is unchanged.
- Hub server code moved from `main.rs` into the library crate (`hub_lib`), with the API route table in `routes.rs` and probe endpoints in `ingest.rs`. No behaviour change.
- Time-dependent hub logic reads an injectable clock (`AppState.clock`) instead of the system clock directly.

### Removed
- The single-column `reports(probe_id)` index, now covered by `reports(probe_id, timestamp)`.
- Unused crypto dependencies from the probe (`x25519-dalek`, `chacha20poly1305`) and the hub (`ed25519-dalek`, `chacha20poly1305`).

## [0.1.0]

- Initial versions of the probe, hub, and `blentinelmake`. See git history.
