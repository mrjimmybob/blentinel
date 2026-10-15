# 0003 · Hub-Managed Probe Configuration & Command Channel

| | |
|---|---|
| **Status** | Accepted. §4.2's response envelope implemented in M2-03 with the changes in D-48 (signing key derived from the hub identity and pinned by the enrollment code; ack sealed to the probe's identity key and bound to the report hash; no `accepted` flag, the HTTP status says it). Checks (§4.1) and the config push implemented in M2-04 with the changes in D-49 (revision = content digest instead of version counters; the interval moves to the hub too, so the probe has no config file; no `[[resources]]` import; no templates or `depends_on` yet). The UI (§4.5) implemented in M2-05 as a page per probe, with "copy checks from another probe" from the bulk actions (D-50); templates are later. The command channel (§4.3) implemented in M2-06 with the changes in D-56 (at-least-once delivery: a command is sent in every answer until its result arrives; expiry worked out on reading; one kind, `run_checks`, behind "Run checks now" on the probe page; protocol v6). §4.2's `probe_health` implemented in M4-02 with the changes in D-73 (uptime, reports held back, clock offset measured by the probe, last problem; the version and config state keep their own fields; protocol v8). §4.3's long-poll implemented in M4-03 with the changes in D-76 (`POST /api/probe/commands`, not `GET /api/probe/wait`: signed and sealed like a report; the answer carries the commands and says when to ask again, so the hub chooses holding or polling; held up to 25 s; protocol stays v8) |
| **Milestone** | M2 — Zero-touch onboarding |
| **Priority** | P0 |
| **Depends on** | 0002 (probe records), D-18 (`protocol_version`) |
| **Decisions** | D-05, D-15 |

## 1. Problem

Checks are defined in `blentinel_probe.toml` **on the client machine**. Adding, changing, or removing a check means remote access to that machine, and managing 50 sites that way doesn't scale. Checks also have no stable identity: history, silences, and alert state are keyed on the `company:probe:name:target` string (D-15).

## 2. Goals

- Checks are created and edited **in the hub UI** and reach the probe automatically.
- Hub → probe instructions (run now, upgrade, send diagnostics) exist **without** the hub ever connecting to the probe.
- The probe keeps working on its last good config when the hub is down.

## 3. Principle check

| | |
|---|---|
| FP-1 | Config and commands ride **only on responses to probe-initiated requests**. ✔ |
| FP-2 | No new dependencies. The config is cached as a file in the probe directory. ✔ |
| FP-3 | The config document is **signed by the hub**, and the probe verifies it against the pinned hub identity. ✔ |
| FP-4 | The cached config is used when the hub is unreachable. ✔ |

## 4. Design

### 4.1 Checks as hub records

```sql
checks (
  id TEXT PRIMARY KEY,                 -- UUID = CheckId (stable forever)
  probe_id TEXT NOT NULL REFERENCES probes(id),
  name TEXT NOT NULL,
  kind TEXT NOT NULL,                  -- ping|http|tcp|tls_cert|dns|service|... (see 0006)
  target TEXT NOT NULL,
  params TEXT NOT NULL DEFAULT '{}',   -- JSON, schema per kind, validated by common::checks
  interval_secs INTEGER,               -- NULL = probe default
  timeout_ms INTEGER,
  enabled INTEGER NOT NULL DEFAULT 1,
  depends_on TEXT REFERENCES checks(id),   -- used by 0005
  template_id TEXT,                    -- if created from a template
  created_at, updated_at, updated_by
);
probe_config_state (probe_id PRIMARY KEY, desired_version INTEGER NOT NULL, applied_version INTEGER);
```

- `common::checks::CheckSpec` is a **typed enum** (one variant per kind, with typed params) with a single `validate()` implementation. The hub UI, the hub API, and the probe all use it (DRY).
- **Templates** (P2): named sets of checks, e.g. "Windows Server baseline" = CPU, memory, all disks, Windows Update service, pending reboot. They apply to one or many probes. Checks keep a link to their template, so "update template → propagate" is possible.

### 4.2 Protocol: report ↔ response

The probe's report gains:

```rust
struct StatusReport {
    protocol_version: u16,
    probe_id: ProbeId,
    config_version: u64,          // version the probe is currently running
    results: Vec<CheckResult>,    // each carries check_id, not name/target
    probe_health: ProbeHealth,    // version, queue depth, uptime, clock (see 0004)
    // ... existing signed/encrypted envelope
}
```

The hub's response (today an empty 200) becomes an encrypted, hub-signed envelope:

```rust
struct ReportAck {
    protocol_version: u16,
    accepted: bool,
    hub_time: DateTime<Utc>,                 // probe can detect clock skew
    config: Option<SignedProbeConfig>,       // present only if desired_version > config_version
    commands: Vec<SignedCommand>,            // pending commands for this probe
    next_tls_spki: Option<String>,           // rotation (0002 §4.5)
    next_hub_key: Option<String>,
    revoked: bool,
}
```

- **Hub signing key:** the hub gains an Ed25519 signing key (alongside its X25519 key). Its fingerprint is included in the enrollment code, so the probe can verify configs and commands independently of transport encryption.
- **Idempotent and ordered:** a config is replaced atomically. The probe writes `config.json.tmp` → fsync → rename, and it reports the version it actually applied.
- **Invalid config:** the probe rejects it (validation in `common`), keeps the previous one, and reports the error in `probe_health`. The hub UI shows "config rejected: …".

### 4.3 Command channel

```sql
probe_commands (
  id TEXT PRIMARY KEY, probe_id TEXT NOT NULL, kind TEXT NOT NULL, args TEXT,
  created_by TEXT, created_at, expires_at,
  state TEXT NOT NULL,       -- queued|delivered|succeeded|failed|expired
  result TEXT, completed_at
);
```

Command kinds (initial): `run_checks_now`, `send_diagnostics` (probe doctor output + last log lines), `upgrade { version }` (0004), `reload_config`, `rotate_identity`, `uninstall` (with confirmation in the UI).

- Commands are delivered in `ReportAck.commands`. Their results come back in the **next report**.
- **Latency:** at most one check-in interval. For interactive actions (e.g. "run now" from the UI), add an optional **long-poll** (P2): the probe keeps an outbound `GET /api/probe/wait` open for up to ~60 s, and the hub answers early when a command is queued. It is still probe-initiated, so it's FP-1 compliant and works through NAT. Proxies that kill idle connections simply degrade to the check-in latency.

### 4.4 The probe's local files after this change

```
blentinel_probe.toml     bootstrap only: hub URLs, data dir, log level   (written by `probe install`)
identity.key             Ed25519 identity
enrollment.json          probe_id, pinned TLS SPKIs, pinned hub keys
config.json              last applied signed config (cache, FP-4)
queue.db                 offline report buffer (existing)
logs/
```

- The **`[[resources]]` section is removed.** Hot reload for the bootstrap file stays (it is small). Check hot-reload is replaced by config pushes.
- **Migration (skipped in pre-alpha, D-30: recreate checks in the UI):** on the first report after upgrade, an existing probe sends its local `[[resources]]` as an "import proposal". The hub creates matching `checks` rows (keeping history by mapping the old name+target keys to the new `CheckId`s) and pushes back the canonical config. From then on the hub is authoritative.
- `probe run --standalone checks.toml` stays as a **developer/test mode** only (no hub, prints results).

### 4.5 UI

- **Probe page → Checks tab:** list, add (form per kind, driven by the `CheckSpec` schema), edit, enable/disable, delete, "run now", last result, config sync state (`in sync` / `pending v12` / `rejected: …`).
- **Bulk actions:** apply a template to many probes, copy checks from one probe to another.

## 5. Tests

- Unit: `CheckSpec` validation for every kind; config signing and verification; version comparison.
- Protocol e2e: create check in hub → next report → probe applies → results carry the new `check_id`; tampered config rejected; invalid config rejected with the previous one kept; hub offline → probe keeps running the cached config; command queued → delivered → result recorded; expired command never delivered.
- Migration: an old probe with `[[resources]]` → checks created, history mapped.

## 6. Docs to update

`docs/reference/checks.md` (becomes a UI + JSON reference), `docs/install/probe.md` (the bootstrap file shrinks), `docs/architecture.md` (protocol), README hot-reload section (probe part rewritten), CHANGELOG.
