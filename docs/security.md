# Security

Blentinel is designed for zero-trust networks: a probe must be able to report from any network without opening inbound ports, and the hub must not believe anything a probe can't prove. This page describes the protections as implemented today. The planned improvements are in [ROADMAP](ROADMAP.md) (M1 users & roles, M2 enrollment and runtime certificate pinning).

## Summary

| Protection | How |
|---|---|
| No inbound access to client networks | Probes only make outbound requests; the hub never connects to a probe (PRINCIPLES FP-1). |
| Only registered probes are accepted | The probe's public key must be listed in `[[probes]]` in the hub config. |
| Reports can't be forged or altered | Every report is signed with the probe's Ed25519 key. Unsigned reports are rejected. |
| Reports can't be read in transit | Every report is sealed for the hub (X25519 + ChaCha20-Poly1305), with or without TLS. |
| Reports can't be replayed | Timestamp window plus "must be newer than the last stored report". |
| Transport encryption | The web UI and API are HTTPS-only by default (self-signed certificate). Probes may use HTTPS with the hub's certificate pinned; by default they report over HTTP, which their end-to-end signing and encryption already protect. |
| Web UI access | Personal accounts with roles; argon2id passwords; sessions in the database, HttpOnly + SameSite=Strict cookie; every route checks a permission. |

## Probe identity and signatures

On first run each probe creates an **Ed25519 key pair** (`identity.key` in the probe's directory). The public key, in hex, **is the probe's ID**. You register it in the hub's `[[probes]]` list.

Before sealing, the probe signs the report's canonical form (`SignableReport`: every field except the signature itself). The hub verifies that signature against the registered key. A report is rejected with **403** if:

- the probe ID isn't registered,
- the report has **no signature** (probe IDs and the hub's key are public, so an unsigned report could come from anyone),
- or the signature doesn't match (report altered, or signed with another key).

Because the whole report is signed, including `company_id`, `site`, `timestamp` and `protocol_version`, none of it can be changed in transit.

## Sealing (payload encryption)

Every report is encrypted for the hub at the application level, independent of TLS:

1. The probe creates a **fresh X25519 key pair for every report**.
2. It derives a shared secret with the hub's long-term X25519 public key.
3. It encrypts the report with **ChaCha20-Poly1305** using that secret and a random 12-byte nonce.
4. It sends `[32-byte ephemeral public key][12-byte nonce][ciphertext + 16-byte tag]`.

Only the holder of the hub's private key (`hub_identity.key`) can open it. The code is `common/src/crypto.rs`, shared by probe and hub.

> **Limitation: no forward secrecy against theft of the hub key.** The probe's key is fresh per report, but the hub's key is long-lived. Anyone who obtains `hub_identity.key` **and** has recorded past traffic can decrypt those past reports. Protect `hub_identity.key` like a password database. Enabling HTTPS adds a transport layer with forward secrecy. (Earlier docs described this as "perfect forward secrecy"; that was inaccurate.)

## Replay protection

A captured report can't be accepted twice (`service/ingest.rs`, protocol v2):

- **Time window:** the signed timestamp must be at most **5 minutes in the future** and at most **7 days old** (the longest a probe buffers a report). Otherwise **422**. Probe clocks must therefore be within 5 minutes of the hub: run NTP.
- **Ordering:** a report must be **strictly newer than the probe's latest stored report**. Otherwise **409**. The check and the insert are one atomic database statement, so two copies arriving at once can't both be stored.
- Both checks run **after** signature verification, so forged traffic learns nothing about them.

Probes deliver buffered reports oldest-first, so a probe returning from an outage passes these checks. (Known limitation: deleting a probe's data in the admin UI also removes its "latest report", so reports captured in the last 7 days could be replayed once afterwards.)

## Hub key handshake

If `hub_public_key` isn't set in the probe config, the probe fetches the hub's X25519 public key from `GET /api/handshake` at startup and trusts whatever it receives (trust on first use, open to interception at that moment). If it **is** set, the probe uses that key and skips the handshake. For the strongest setup, set `hub_public_key` (from `hub --print-public-key`) or use HTTPS with a pinned certificate. Enrollment codes (ROADMAP M2) will make this automatic.

## HTTPS and certificate pinning

By default (split mode, D-43) the hub serves the web UI and API only over HTTPS, with an auto-generated self-signed certificate (ECDSA P-256, valid 10 years). Its plain-HTTP port serves only the two probe endpoints, whose payloads are signed and encrypted end to end, and redirects everything else to HTTPS, so passwords and session cookies never cross plain HTTP. Probes built with that certificate (`probe/hub_cert.pem`, embedded at compile time) trust **only** it, not the operating system's certificate authorities. That protects against interception even by someone holding a certificate from a trusted CA.

Today a certificate change requires rebuilding the probe. Runtime pinning via enrollment codes is planned (D-01). Setup steps are in [install/https.md](install/https.md).

## Web interface

- **Accounts:** one per person, with a role (Owner, Admin, Technician, Viewer). See [guide/users.md](guide/users.md).
- **First run:** a hub without users accepts only a one-time setup code (logged, and written owner-only to `setup.code`) to create the first Owner, so a freshly installed hub can't be claimed by someone else.
- **Passwords:** at least 12 characters, stored as argon2id hashes. An unknown username and a wrong password get the same answer after the same work. Temporary passwords (new users, resets) must be changed at the next login; until then every route except *who am I*, *change password* and *log out* answers `403 password_change_required`.
- **Sessions:** 256 random bits in an `HttpOnly; SameSite=Strict` cookie. The database stores only its SHA-256, so a copy of the database can't be used to take over a session. A session ends after 12 idle hours or 7 days, at logout, or when the user is deactivated or their password is reset or changed (other sessions). Sessions survive a hub restart.
- **Authorization:** every API route declares its access in one table (`hub/src/routes.rs`), enforced before the handler runs; a test walks every route with every role.
- **Company scope:** Technicians, Viewers and Clients see only their companies (all, only some, or all except some). Lists are filtered in the storage layer; single items outside the scope answer `404`, never `403`, so a customer can't discover other customers. Clients don't get internal notes or contacts. A test lists every route open to scoped users and fails if a new one has no scoping test. The probe endpoints authenticate cryptographically instead.
- **Archives** never contain accounts or sessions.
- **HTTPS:** the UI is HTTPS-only by default (above), and session cookies are marked `Secure` whenever the request came over HTTPS (or `behind_proxy` is set).
- **CSRF:** besides `SameSite=Strict`, every POST except the probe endpoints must carry the header `X-Blentinel: 1`. A browser can't add it to a cross-site form, and a cross-site `fetch` with it needs a CORS preflight the hub never grants. Otherwise: `403 csrf`.
- **Password guessing is slowed down:** per client address and username, 5 failures are free, then each failure locks that pair for 30 s, doubling up to 15 min; a success clears it. 20 failures from one address within an hour (any usernames) lock that address for 15 min. Locked attempts get `429 too_many_attempts` with `Retry-After`, without the password being checked. Wrong setup codes count the same way. The counters are in memory (a restart clears them).
- **Audit log:** logins (and failed ones, with the username tried and the client address), user and company changes, silences, data deletion and archiving are recorded with who, when and from where ([guide/audit.md](guide/audit.md)). Admins read it; it never contains passwords or tokens, and it isn't copied into archives. Writing it is best effort: a failure is logged but never blocks the action.

## Files to protect

| File | Where | If leaked | If lost |
|---|---|---|---|
| `hub_identity.key` | hub | captured reports can be decrypted; the hub can be impersonated | probes must redo the handshake; buffered reports become unreadable |
| `blentinel.db` | hub | password hashes (argon2id, slow to attack) and all monitoring data; session hashes are useless | restore from backup; `hub user create` makes a new Owner |
| `setup.code` | hub, before the first Owner exists | whoever has it can create the first Owner | restart the hub for a new code |
| `hub_tls_key.pem` | hub (HTTPS) | the hub can be impersonated over HTTPS | a new certificate means rebuilding probes |
| `identity.key` | each probe | reports can be forged as that probe | the probe gets a new ID and must be re-registered |

On Linux and macOS the hub and probe create these files readable only by their owner (mode `0600`). On Windows they inherit the folder's permissions, and by default every local user can **read** files under `C:\ProgramData` and new folders under `C:\`. On a machine with other users, restrict the folder to SYSTEM and administrators, e.g. `icacls "C:\ProgramData\Blentinel\Hub" /inheritance:r /grant:r "SYSTEM:(OI)(CI)F" "Administrators:(OI)(CI)F"` (the same for `C:\Blentinel\probe`). The installers don't do this yet (ROADMAP M2-08). Files created by versions before 2026-09-27 kept the default `0644`. The Linux installers' folders (`/var/lib/blentinel`, `/opt/blentinel-probe`) already blocked other users, but tighten them anyway: `chmod 600 identity.key` (probe) and `chmod 600 hub_identity.key` (hub).

None of these belong in version control. `.gitignore` excludes them.

## Reporting a vulnerability

Please report security issues privately to the maintainer rather than in a public issue.
