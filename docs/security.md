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
| Optional transport encryption | HTTPS with the hub's certificate pinned in the probe. |
| Web UI access | Admin token login with an HttpOnly session cookie. |

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

## HTTPS and certificate pinning (optional)

With `[server.tls] enabled = true`, the hub serves HTTPS with an auto-generated self-signed certificate (ECDSA P-256, valid 10 years). Probes built with that certificate (`probe/hub_cert.pem`, embedded at compile time) trust **only** it, not the operating system's certificate authorities. That protects against interception even by someone holding a certificate from a trusted CA.

Today a certificate change requires rebuilding the probe. Runtime pinning via enrollment codes is planned (D-01). Setup steps are in [install/https.md](install/https.md).

## Web interface

- **Login:** the admin token from `hub_auth.token` (printed on first start; reset with `hub --reset-admin-token`).
- **Session:** a random session ID in an `HttpOnly; SameSite=Lax` cookie, valid 24 hours. Sessions are held in memory, so a hub restart logs everyone out.
- Every dashboard and admin endpoint requires a session. The probe endpoints authenticate cryptographically instead.
- Planned (ROADMAP M1): user accounts and roles, sessions stored in the database, HTTPS for the UI by default.

## Files to protect

| File | Where | If leaked | If lost |
|---|---|---|---|
| `hub_identity.key` | hub | captured reports can be decrypted; the hub can be impersonated | probes must redo the handshake; buffered reports become unreadable |
| `hub_auth.token` | hub | full admin access to the UI | reset with `hub --reset-admin-token` |
| `hub_tls_key.pem` | hub (HTTPS) | the hub can be impersonated over HTTPS | a new certificate means rebuilding probes |
| `identity.key` | each probe | reports can be forged as that probe | the probe gets a new ID and must be re-registered |

None of these belong in version control. `.gitignore` excludes them.

## Reporting a vulnerability

Please report security issues privately to the maintainer rather than in a public issue.
