# Security

Blentinel is designed for zero-trust networks: a probe must be able to report from any network without opening inbound ports, and the hub must not believe anything a probe can't prove. This page describes the protections as implemented today. The planned improvements are in [ROADMAP](ROADMAP.md) (M1 users & roles, M2 enrollment and runtime certificate pinning).

## Summary

| Protection | How |
|---|---|
| No inbound access to client networks | Probes only make outbound requests; the hub never connects to a probe (PRINCIPLES FP-1). |
| Only enrolled probes are accepted | A probe joins with a short-lived enrollment code from the hub's UI; the hub keeps a record per probe and can revoke it. |
| The hub decides who a probe reports for | The company and site come from the probe's record, never from the report. |
| Probes trust only their hub | The enrollment code pins the hub's encryption key, signing key and TLS certificate, with no trust-on-first-use step. |
| The hub's answers can't be forged | Each answer to a report is signed by the hub, sealed for that probe, and bound to that report. |
| Reports can't be forged or altered | Every report is signed with the probe's Ed25519 key. Unsigned reports are rejected. |
| Reports can't be read in transit | Every report is sealed for the hub (X25519 + ChaCha20-Poly1305), with or without TLS. |
| Reports can't be replayed | Timestamp window plus "must be newer than the last stored report". |
| Transport encryption | The web UI and API are HTTPS-only by default (self-signed certificate). Probes use HTTPS pinned to the hub's certificate, unless enrolled with an `http://` address (their reports are still signed and encrypted end to end). |
| Web UI access | Personal accounts with roles; argon2id passwords; sessions in the database, HttpOnly + SameSite=Strict cookie; every route checks a permission. |

## Probe identity and signatures

On first run each probe creates an **Ed25519 key pair** (`identity.key` in the probe's directory). The public key, in hex, **is the probe's ID**. Enrolling creates the hub's record for that ID (next section).

Before sealing, the probe signs the report's canonical form (`SignableReport`: every field except the signature itself). The hub verifies that signature against the enrolled key. A report is rejected with **403** if:

- the probe ID isn't enrolled, or its record was revoked,
- the report has **no signature** (probe IDs and the hub's key are public, so an unsigned report could come from anyone),
- or the signature doesn't match (report altered, or signed with another key).

Because the whole report is signed, including `timestamp` and `protocol_version`, none of it can be changed in transit. The report's own `company_id` and `site` are ignored: the hub files it under the company and site in the probe's record, so a probe can't report for another customer.

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

## Enrollment

A probe joins a hub with an **enrollment code** made by **Add probe** in the UI (Owner, Admin, or a Technician for their own companies). The code, `BLT1-…`, contains:

- the hub's address;
- the hub's **X25519 public key**, in full, so the probe never has to ask the hub for it (no trust on first use);
- the **SHA-256 fingerprint of the hub's TLS certificate**, when the address is `https://` and the hub serves TLS itself;
- a token ID and a 128-bit secret, and a checksum that catches typos.

`probe enroll <code>` sends the token, the probe's ID and some facts about the machine (hostname, OS, version) to `POST /api/enroll`, **signed** with the probe's key and **sealed** for the hub's key, over HTTPS pinned to that fingerprint. The hub stores only a SHA-256 hash of each secret and redeems the token atomically, so a single-use code can't be used twice even by two probes at once. A code works only until it expires (24 hours by default, 30 days at most) or runs out of uses (1 by default), and can be revoked earlier. Wrong codes count against the address like failed logins.

The code decides the probe's **company and site**. The hub stores them in the probe's record and files every report under them, whatever the report says. Treat a code like a password until it's used: whoever has it can join a probe to that company.

**Revoke** (Owner and Admin) turns a probe away for good: its reports get `403 probe_revoked` with a signed answer saying so, and the probe stops reporting, clears its buffer and waits. It can only come back by enrolling with a new code.

## Probe downloads

**Add probe** gives a one-line command that downloads the probe from the hub and installs it (D-53). The download itself isn't trusted: it accepts any certificate (`curl -k`; PowerShell skips the check) and works over plain HTTP, because a new machine can't know the hub's self-signed certificate yet. What makes it safe is the **SHA-256 in the command**: the admin copies it from the hub's HTTPS page, and the command runs the file only if its hash matches (`sha256sum -c` on Linux; on Windows the hash is compared before anything runs, with `$ErrorActionPreference='Stop'`, so a failed download or a mismatch runs nothing). From there `probe install --code` pins the hub's certificate and keys from the code, as `probe enroll` does. Anyone can download the binaries (`/download/probe/{target}`, no login); they hold no secrets.

## The hub's answers

The hub answers every authenticated report with a `ReportAck` ([reference/protocol.md](reference/protocol.md#the-hubs-answer)). Later milestones put the probe's checks and commands in it, so it is protected like the reports:

- **Signed** with the hub's **Ed25519 signing key**. That key is derived from `hub_identity.key` (domain-separated SHA-256), so there is still one key file to protect and back up. Its public half is in every enrollment code. Anyone can seal a message for a probe, because its key is public, but only the hub can sign one.
- **Sealed** for the probe, to the X25519 form of its identity key, so nobody else can read it, even over plain HTTP.
- **Bound** to the report it answers (its SHA-256), so a recorded answer can't be replayed later.

The probe acts on an answer only after these checks. A revocation in a plain `403`, which anyone on the network path could send, only drops that one report.

## HTTPS and certificate pinning

By default (split mode, D-43) the hub serves the web UI and API only over HTTPS, with an auto-generated self-signed certificate (ECDSA P-256, valid 10 years). Its plain-HTTP port serves only the two probe endpoints, whose payloads are signed and encrypted end to end, and redirects everything else to HTTPS, so passwords and session cookies never cross plain HTTP.

A probe enrolled with an `https://` address trusts **only** the certificate whose fingerprint is in its code, not the operating system's certificate authorities. That protects against interception even by someone holding a certificate from a trusted CA. The TLS handshake is still verified normally, so the server must hold that certificate's private key. When the certificate changes, probes must be enrolled again (automatic rotation is ROADMAP M2-13). Setup: [install/https.md](install/https.md).

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
| `hub_identity.key` | hub | captured reports can be decrypted; the hub's answers can be forged (its signing key is derived from this file) | every probe must be enrolled again; buffered reports become unreadable |
| `blentinel.db` | hub | password hashes (argon2id, slow to attack) and all monitoring data; session hashes are useless | restore from backup; `hub user create` makes a new Owner |
| `setup.code` | hub, before the first Owner exists | whoever has it can create the first Owner | restart the hub for a new code |
| `hub_tls_key.pem` | hub (HTTPS) | the hub can be impersonated over HTTPS | a new certificate means enrolling HTTPS probes again |
| `identity.key` | each probe | reports can be forged as that probe (revoke it in the hub) | the probe gets a new ID and must be enrolled again |
| an enrollment code | until used or expired | a machine can join as a probe of that company and site | create another |

On Linux and macOS the hub and probe create these files readable only by their owner (mode `0600`). On Windows they inherit the folder's permissions, and by default every local user can **read** files under `C:\ProgramData` and new folders under `C:\`. **`hub install` restricts the hub's data folder** (`C:\ProgramData\Blentinel\Hub`) to SYSTEM and Administrators, inherited by everything in it (D-52); on Linux it makes `/var/lib/blentinel` `0700` for the service user and the config `0640 root:blentinel`. `hub doctor` warns if the folder is open to others. **`probe install` does the same for the probe's folder** (D-53): `C:\Program Files\Blentinel\Probe` for SYSTEM and Administrators only; on Linux `/var/lib/blentinel-probe`, `0700`, owned by the service's throwaway user (systemd `DynamicUser`). `probe doctor` warns if the folder is open to others. A probe run by hand (`probe enroll`, `./probe`) keeps its files in its own folder: restrict it yourself on a machine with other users. A `hub backup` holds the hub's private key: it is written `0600` (owner only); keep it like the key. Files created by versions before 2026-09-27 kept the default `0644`. Folders set up by the old installer scripts already blocked other users; reinstall with `hub install` / `probe install` anyway.

None of these belong in version control. `.gitignore` excludes them.

## Reporting a vulnerability

Please report security issues privately to the maintainer rather than in a public issue.
