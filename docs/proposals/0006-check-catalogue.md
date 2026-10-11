# 0006 · Check Catalogue — More Metrics, Natively

| | |
|---|---|
| **Status** | Proposed |
| **Milestone** | M5 — Monitoring depth |
| **Priority** | per check, see table |
| **Depends on** | 0003 (typed `CheckSpec`, hub-managed config) |
| **Old TODO folded in** | "Create database plugin module"; PROGRESS "more models (databases, printers, NAS)" |

## 1. Rules every check follows

1. **Native Rust, no shelling out** (FP-2). There is one allowed exception, opt-in script checks (§4), which are off by default.
2. **Typed parameters** in `common::checks::CheckSpec` with a single `validate()` (DRY; used by hub and probe).
3. **Timeout on every check** (default 5 s, configurable), and every check runs in its own task. A slow check can never stall the cycle.
4. **Result model change:** multiple metrics per result.
   ```rust
   struct CheckResult { check_id, status: RawStatus, message, latency_ms: Option<u32>,
                        metrics: Vec<Metric> }       // Metric { key, value: f64, unit }
   ```
   One check can report several values: disk used % and free GB, ping latency and loss and jitter, toner cyan/magenta/yellow/black. This replaces the single `metric_value`/`metric_unit` pair.
5. **Raw status only on the probe.** The probe reports "succeeded / failed + metrics". Warning and Down thresholds are evaluated on the hub (0005), so they can change without pushing config.
6. Every check has **tests** against a local fake (e.g. an in-test HTTP server, a TCP listener, an SNMP agent simulator) and a **reference entry** in `docs/reference/checks.md`.

## 2. Improvements to existing checks

| Check | Today | Improvement | Priority |
|---|---|---|---|
| **ping** | One echo, Up/Down | N packets (default 4) → **packet loss %, avg/max latency, jitter**; TCP-connect fallback when ICMP isn't permitted | P1 |
| **http** (done: M5-02, D-78, but for basic auth) | Status 2xx/any? + latency | Method, **expected status codes**, **keyword present/absent**, max response time, follow redirects, custom headers, basic auth, "accept invalid cert" toggle | P1 |
| **tcp** | Fixed 3 s timeout | Configurable timeout; optional banner match | P1 |
| **local_disk** | Configured volumes | **Auto-discover all fixed volumes**; used %, free GB; ignore-list | P1 |
| **local_cpu / mem / uptime** | Basic | Load average (Unix), swap/pagefile; uptime also detects **unexpected reboots** (uptime went down) | P2 |

## 3. New checks, ranked (value to an MSP ÷ effort)

| # | Check | What it answers | Implementation notes | Priority |
|---|---|---|---|---|
| 1 | **TLS certificate** (done: M5-01, D-77) | "Does a cert expire within 30/7 days? Wrong hostname? Weak chain?" | rustls handshake to host:port (any TLS service: HTTPS, SMTPS, RDP gateway…); days-left metric | **P1** |
| 2 | **File / folder age & size** | "Did last night's **backup** write a file?" | Newest-file mtime under a path + glob; size min | **P1** |
| 3 | **Service status** | "Is the backup agent / SQL / print spooler running?" | Windows: SCM API (`windows-service` crate, already a dependency). Linux: systemd over D-Bus (`zbus`, pure Rust). Optional "restart attempt" is **not** in scope (read-only monitoring). | **P1** |
| 4 | **DNS** | "Does the name resolve, to the expected IP, from this network?" | `hickory-resolver`; specific resolver optional; record type | **P1** |
| 5 | **Process running** | "Is X running? How many instances? CPU/mem of it?" | `sysinfo` (already a dependency) | P2 |
| 6 | **SNMP v2c/v3** (generic OID get/walk) | Switches, firewalls, UPS, NAS, printers | Pure-Rust SNMP crate. v3 auth/priv. Values → metrics. | **P2** (high value) |
| 7 | **SNMP presets** | Printer toner/paper/status (Printer-MIB), interface up/down + errors + throughput (IF-MIB), Synology/QNAP volume & RAID & temperature, APC UPS battery/runtime | Presets = named bundles of OIDs + thresholds, shipped as data | P2 |
| 8 | **Database liveness** | "Is SQL Server / MySQL / PostgreSQL accepting connections?" | Protocol-level handshake **without credentials** (TDS pre-login, MySQL greeting, PG SSLRequest). Optional authenticated `SELECT 1` later (P3, needs secret storage). | P2 |
| 9 | **Pending updates and reboot** (Windows and Linux; monitor only, D-59) | "Which machines have security updates waiting? Which need a reboot? When did the OS last look?" | Windows: registry keys and the WUA COM API (pure Win32). Linux: PackageKit over D-Bus where present, else the distribution's status files (`/var/run/reboot-required`, Ubuntu's `updates-available`). Never installs anything | P2 |
| 10 | **Windows Event Log** | "Any Event ID 7/51/153 (disk errors) or backup failures in the last N min?" | EvtQuery Win32 API; filter by log/level/ID/source | P2 |
| 11 | **Network interface** | Throughput, errors, link state of the probe host | `sysinfo` | P2 |
| 12 | **Temperature / SMART** | Hardware health on the probe host | `sysinfo` components; SMART needs admin & platform APIs | P3 |
| 13 | **Mail**: SMTP/IMAP/POP3 banner + STARTTLS | "Is the mail server up?" | TCP + protocol greeting; TLS-cert check reuses #1 | P3 |
| 14 | **NTP offset** | "Is this network's clock drifting?" | SNTP query | P3 |
| 15 | **UPS via NUT** | Battery, on-battery events | NUT TCP protocol (3493) | P3 |
| 16 | **Hypervisor/VM status** | Proxmox / Hyper-V / ESXi VM states | HTTPS APIs (Proxmox, ESXi); Hyper-V via WMI (P3) | P3 |
| 17 | **LDAP/AD bind** | "Is the domain controller answering?" | Anonymous bind / rootDSE read | P3 |
| 18 | **Script check (opt-in)** | Anything custom | Executes a user-provided script. **Disabled by default**, must be enabled per probe in its local bootstrap file (so a hub compromise can't turn it on remotely). Exit code → status, stdout → message/metrics. This is the only sanctioned exception to "no shelling out" and must be documented as such. | P3 |

## 4. Discovery (P2)

- **Host inventory at enrollment:** the probe reports OS, volumes, running services, and network interfaces. The hub offers "suggested checks" (all disks, key services, gateway ping) with one-click accept.
- **Network discovery (P3):** a probe-side ping/ARP sweep and common-port probe of its local subnet, on demand via command (0003). Results are shown as "discovered devices → add checks". It scans only the probe's own LAN, outbound from the probe (FP-1 compliant). It is off by default because some clients forbid scanning.

## 5. Templates (P2, with 0003)

Ship built-in templates as data files: *Windows Server baseline*, *Linux server baseline*, *Domain controller*, *Printer (SNMP)*, *Synology NAS*, *Website* (HTTP + TLS + DNS), *Backup freshness*. Admins can clone and edit them.

## 6. Principle check

FP-2 is the constraint for this whole proposal. Every new crate must be pure Rust (or statically linkable without system libraries) and pass the CI static-linkage check. Checks that need elevated privileges declare it, so the UI can warn ("requires admin/root on the probe").

## 7. Docs to update

`docs/reference/checks.md`: one section per check (purpose, parameters, metrics produced, privileges, example). Generated tables where possible from `CheckSpec` metadata (DRY). Plus CHANGELOG per added check.
