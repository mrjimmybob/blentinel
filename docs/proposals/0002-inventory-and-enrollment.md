# 0002 · Companies, Sites & Zero-Touch Probe Enrollment

| | |
|---|---|
| **Status** | Accepted. §4.1 companies and sites implemented in M1-03 (D-44); enrollment §4.2–4.4 implemented in M2-01 with the changes in D-47 (whole hub key in the code, certificate SHA-256 instead of SPKI, no `[[probes]]` import, no approval step yet). Rotation (§4.5) implemented in M2-13 with the changes in D-61 (the TLS pin is the certificate's public key again; the hub switches when every probe holds the next pins or when an admin says so, and only flags an overdue rotation; the certificate is generated, not uploaded, until ACME) |
| **Milestone** | M1 (companies & sites, §4.1) · M2 (enrollment) |
| **Priority** | P0 |
| **Depends on** | 0001 (users/audit), M0 (migrations, `protocol_version`) |
| **Decisions** | D-01, D-02, D-04, D-17 |

## 1. Problem

- **Companies and sites aren't records.** A company is whatever `company_id` string a probe sends, so a typo creates a new company. `site` is free text in every report. There is nowhere to keep a display name, contacts, notes, or timezone.
- **Adding a probe takes 5 manual steps:** start the probe → copy the public key from its console → edit the hub TOML → save → restart the probe. HTTPS additionally needs a **custom probe build** with the hub cert compiled in.
- The probe trusts whatever key `/api/handshake` returns on first contact (D-02).

## 2. Goals

- Companies → Sites → Probes managed in the UI.
- Adding a probe = click **Add probe** in the UI, run **one command** on the client machine, done.
- The hub decides which company/site a probe belongs to (FP-3).
- The same official probe binary works for every hub (no per-hub build).

## 3. Principle check

| | |
|---|---|
| FP-1 | Enrollment is a **probe-initiated** request. The hub never contacts the probe. ✔ |
| FP-2 | No new probe dependencies. Pinning uses rustls custom verification. ✔ |
| FP-3 | Stronger: pinned hub key and cert from first contact; the hub assigns the tenant; tokens are single-use and expire. ✔ |
| PP-1/PP-2 | One command; no TOML edits or key copying. ✔ |

## 4. Design

### 4.1 Data model

```sql
companies (
  id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
  notes TEXT, contact_name TEXT, contact_email TEXT, contact_phone TEXT,
  created_at DATETIME NOT NULL, archived_at DATETIME
);
sites (
  id TEXT PRIMARY KEY, company_id TEXT NOT NULL REFERENCES companies(id),
  name TEXT NOT NULL, address TEXT, timezone TEXT NOT NULL DEFAULT 'UTC', notes TEXT,
  UNIQUE(company_id, name)
);
probes (
  id TEXT PRIMARY KEY,                   -- UUID (hub-assigned)
  public_key TEXT NOT NULL UNIQUE,       -- Ed25519 hex
  name TEXT NOT NULL,
  company_id TEXT NOT NULL REFERENCES companies(id),
  site_id TEXT REFERENCES sites(id),
  state TEXT NOT NULL,                   -- pending | active | revoked
  hostname TEXT, os TEXT, arch TEXT, probe_version TEXT,
  enrolled_at DATETIME, last_seen_at DATETIME, revoked_at DATETIME
);
enrollment_tokens (
  id TEXT PRIMARY KEY, secret_hash TEXT NOT NULL,
  company_id TEXT NOT NULL, site_id TEXT, probe_name TEXT,
  created_by TEXT REFERENCES users(id), created_at, expires_at,
  max_uses INTEGER NOT NULL DEFAULT 1, uses INTEGER NOT NULL DEFAULT 0,
  require_approval INTEGER NOT NULL DEFAULT 0
);
```

`probe_heartbeats` merges into `probes` (liveness = `last_seen_at` + a derived state).

### 4.2 The enrollment code

A compact, copy-pasteable string, e.g. `BLT1-7Q4M-…` (base32, checksummed), that encodes:

| Field | Purpose |
|---|---|
| format version | forward compatibility |
| hub URL(s) | primary plus optional fallback (e.g. LAN and public) |
| hub X25519 key fingerprint | verifies `/api/handshake` (fixes D-02) |
| hub TLS SPKI SHA-256 | certificate pinning without rebuilding (fixes D-01) |
| token ID + secret | authorizes this one enrollment |

Defaults: expires after **24 h** and is **single-use**. A multi-use code can be generated for mass deployment, e.g. via RMM or GPO: "valid 7 days, 50 uses, require approval".

### 4.3 Flow

```
Admin (UI)                         Hub                                  Probe (client network)
   | Add probe → site "HQ"          |                                        |
   |------------------------------->| create enrollment_token               |
   |<-- code + OS one-liners -------|                                        |
   |                                |                                        |
   |       technician runs: probe install --code BLT1-…                      |
   |                                |<-- GET /api/handshake -----------------| verify TLS SPKI pin
   |                                |--- hub X25519 key -------------------->| verify key fingerprint
   |                                |<-- POST /api/enroll (encrypted+signed)-| {token, probe_pubkey,
   |                                |    verify token, create probe record  |  hostname, os, arch, version}
   |                                |--- {probe_id, name, company, site, --->| persist enrollment,
   |                                |     initial config (0003)}            | install service, start
   |  probe appears "active" (or "pending approval")                         |
```

- `require_approval` → the probe is `pending`: its reports are accepted but not shown or alerted on until an admin approves it. Useful for mass-deploy codes.
- **Re-enrollment** (machine rebuilt, new identity): a new code enrolls the new key. The UI offers "Replace existing probe X", which keeps its checks and history and revokes the old key.
- **Revocation:** state `revoked`. Further reports are rejected, and the response tells the probe it has been revoked so it can stop cleanly and log why.

### 4.4 Tenant authority

- Reports are attributed to `probes.company_id` and `probes.site_id`. The `company_id`, `site`, and `hostname` fields in the report are **informational only**. Hostname updates the record; company and site are ignored (D-17).
- Moving a probe to another site or company is a UI action (audited).

### 4.5 Certificate and key rotation

- **TLS cert rotation:** the admin uploads or generates the new cert → for a configurable overlap window (default 14 days), report responses carry `next_tls_spki` → probes add it to their pin set → the hub switches → responses stop advertising the old one → probes drop it.
- **Hub X25519 key rotation:** same pattern (`next_hub_key`).
- Both are handled by the protocol, never by rebuilding the probe.

### 4.6 Migration of existing installations: **not needed in pre-alpha (D-30)**. Kept below for when external users exist.

On first start of the new version:

1. Each distinct `company_id` in `probe_heartbeats`/`reports` → a `companies` row (slug = old id, name = old id).
2. Each distinct `(company_id, site)` → a `sites` row.
3. Each `[[probes]]` entry in the TOML → a `probes` row (`active`), linked to its company and site from its latest report.
4. The `[[probes]]` TOML section is ignored from then on, with a warning to remove it.

Existing probes keep working **without re-enrollment** (same keys). Existing custom probe builds with an embedded cert keep working. The next probe upgrade (0004) switches them to runtime pinning.

### 4.7 UI

- **Companies** list (search, status, counts) → **Company** page (details, contacts, notes, sites, probes, uptime) → **Site** page (probes, checks).
- **Add probe** wizard: pick site → choose options (expiry, uses, approval) → shows the code and **copy-ready commands per OS** (see 0004) → live "waiting for probe…" indicator that turns green when enrollment arrives.
- **Pending probes** inbox for approvals.
- Probe page: rename, move, replace, revoke, and version/OS/hostname details.

## 5. Tests

- Unit: enrollment code encode/decode round-trip, checksum detection, expiry, max-uses.
- Integration: enroll success; expired, reused, and forged tokens rejected; tampered TLS or hub-key fingerprint → probe aborts; revoked probe rejected; report `company_id` spoofing ignored.
- Migration test: an old-schema DB fixture + old TOML → correct companies, sites, and probes.
- E2E: UI "Add probe" → in-process probe enrolls → appears active.

## 6. Docs to update

`docs/install/probe.md` (the new one-command flow replaces README Steps 2–5), `docs/guide/companies-sites-probes.md`, `docs/security.md` (enrollment and rotation), README quick start, remove "HTTPS: copy cert and rebuild probe" instructions, CHANGELOG.
