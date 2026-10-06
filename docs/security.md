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
- the hub's **Ed25519 signing key**, which signs its answers;
- the **SHA-256 of the public key of the hub's TLS certificate**, when the address is `https://` and the hub serves TLS itself (D-61: the key, not the whole certificate, so a renewal with the same key keeps the pin);
- a token ID and a 128-bit secret, and a checksum that catches typos.

`probe enroll <code>` sends the token, the probe's ID and some facts about the machine (hostname, OS, version) to `POST /api/enroll`, **signed** with the probe's key and **sealed** for the hub's key, over HTTPS pinned to that key. The hub stores only a SHA-256 hash of each secret and redeems the token atomically, so a code limited to one use can't be used twice even by two probes at once. A code works only until it expires (24 hours by default, 30 days at most) or reaches its limit of uses if it has one (none by default, D-54), and can be revoked earlier. Wrong codes count against the address like failed logins.

The code decides the probe's **company and site**. The hub stores them in the probe's record and files every report under them, whatever the report says. Treat a code like a password until it expires or is revoked: whoever has it can join probes to that company. Each enrollment is in the audit log and the probe appears on the company's page, where it can be revoked.

**Rotating the hub's certificate or key** (D-61, admin page **Hub keys**): the next ones are announced in the hub's answers, **signed with the current key**, so only the hub can give probes new pins; a man in the middle can't, even on plain HTTP. The hub switches when every probe holds them (or when an admin says so), keeps the previous key only as long as buffered reports may need it, and revokes enrollment codes made before. This is routine replacement, not a response to a **stolen key**: whoever has the key can sign a rotation too. After a theft, give the hub a new key without rotation (a new `identity_key_path`, or delete the key with the hub stopped) and enroll every probe again with new codes.

**Revoke** (Owner and Admin) turns a probe away for good: its reports get `403 probe_revoked` with a signed answer saying so, and the probe stops reporting, clears its buffer and waits. It can only come back by enrolling with a new code.

## Probe downloads

**Add probe** gives a one-line command that downloads the probe from the hub and installs it (D-53). The download itself isn't trusted: it accepts any certificate (`curl -k`; PowerShell skips the check) and works over plain HTTP, because a new machine can't know the hub's self-signed certificate yet. What makes it safe is the **SHA-256 in the command**: the admin copies it from the hub's HTTPS page, and the command runs the file only if its hash matches (`sha256sum -c` on Linux; on Windows the hash is compared before anything runs, with `$ErrorActionPreference='Stop'`, so a failed download or a mismatch runs nothing). From there `probe install --code` pins the hub's certificate and keys from the code, as `probe enroll` does. Anyone can download the binaries (`/download/probe/{target}`, no login); they hold no secrets. The hub offers only binaries of releases it checked against its release keys ([below](#probe-updates)).

## Installing the hub

The one-line install (`hub.sh`, D-57) trusts the **release key**, not the download host: the script carries the release public keys, checks the Ed25519 signature of `SHA256SUMS` with `openssl`, then each file's SHA-256, and installs nothing if any check fails. A mirror (`BLENTINEL_RELEASE_URL`) or a changed file on the release host is therefore refused. The script itself comes over HTTPS from GitHub; to avoid trusting that either, download it, read it and compare it with the release's `SHA256SUMS` before running it. The hub then checks the shipped probe release against the keys compiled into it, as for any release.

## Probe updates

Probes update themselves from their hub (D-41, D-55), so what a probe runs must not depend on trusting the hub:

- **Releases are signed with a release key that never reaches the hub.** `blentinelmake release keygen` creates it on the build machine; its public half is compiled into every hub and probe built there. A probe installs a release only if its `manifest.json` is signed with one of the keys it was built with, so a hacked hub, or anyone between it and the probe, can at worst **withhold** an update, never deliver one. The hub refuses releases not signed with its own copy of the keys, so a wrong upload fails at once.
- **The manifest pins every binary** by SHA-256 and size; the probe checks the download against it and never reads more than the stated size. File names are fixed (`probe-<target>`), so a manifest can't point anywhere else.
- **No downgrades:** a probe refuses a version older than its own, so an old release with a known flaw can't be replayed onto it.
- **Nothing is offered until an admin approves a version** (`ManageData`, audited), and `probe updates off` lets the owner of a machine refuse updates locally.
- **A version that doesn't work is undone:** it must pass `--self-test` before anything is replaced, then deliver a report within 15 minutes and in at most 3 starts, or the previous binary comes back.
- **Key rotation:** put the new key's `.pub` beside the old one and build a release: probes on it trust both keys, and later releases can be signed with the new one. **If the private key leaks**, everything built with it trusts it: build with a new key only (remove the old `.pub`) and install those probes by hand. Keep the private key off shared machines, backed up, or on a hardware token.

## The hub's answers

The hub answers every authenticated report with a `ReportAck` ([reference/protocol.md](reference/protocol.md#the-hubs-answer)). It carries the probe's checks, update offers and commands (D-49, D-55, D-56), so it is protected like the reports:

- **Signed** with the hub's **Ed25519 signing key**. That key is derived from `hub_identity.key` (domain-separated SHA-256), so there is still one key file to protect and back up. Its public half is in every enrollment code. Anyone can seal a message for a probe, because its key is public, but only the hub can sign one.
- **Sealed** for the probe, to the X25519 form of its identity key, so nobody else can read it, even over plain HTTP.
- **Bound** to the report it answers (its SHA-256), so a recorded answer can't be replayed later.

The probe acts on an answer only after these checks. So only the hub can give a probe a command, and a command can only ask for what the probe implements (today: run its checks now); an unknown kind is refused, not guessed at. A revocation in a plain `403`, which anyone on the network path could send, only drops that one report.

## HTTPS and certificate pinning

By default (split mode, D-43) the hub serves the web UI and API only over HTTPS, with an auto-generated self-signed certificate (ECDSA P-256, valid 10 years). Its plain-HTTP port serves only the two probe endpoints, whose payloads are signed and encrypted end to end, and redirects everything else to HTTPS, so passwords and session cookies never cross plain HTTP.

A probe enrolled with an `https://` address trusts **only** a certificate whose public key is pinned in its code (or announced by the hub while it rotates, D-61), not the operating system's certificate authorities. That protects against interception even by someone holding a certificate from a trusted CA. The TLS handshake is still verified normally, so the server must hold that key. The certificate's name and expiry aren't checked: the pin is stronger, and the hub replaces the certificate by rotation (admin page, **Hub keys**). Setup: [install/https.md](install/https.md).

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
| `hub_identity.next.key`, `hub_tls_key.next.pem` | hub, during a rotation (D-61) | as the key they replace, once the hub switches | the rotation can't switch: cancel it and start another |
| `hub_identity.prev.key` | hub, up to 7 days after a key switch | reports sealed for the old key can be decrypted | reports probes sealed before they switched can't be opened |
| `blentinel.db` | hub | password hashes (argon2id, slow to attack) and all monitoring data; session hashes are useless | restore from backup; `hub user create` makes a new Owner |
| `setup.code` | hub, before the first Owner exists | whoever has it can create the first Owner | restart the hub for a new code |
| `hub_tls_key.pem` | hub (HTTPS) | the hub can be impersonated over HTTPS | a new certificate means enrolling HTTPS probes again (rotate instead while it's still there) |
| `identity.key` | each probe | reports can be forged as that probe (revoke it in the hub) | the probe gets a new ID and must be enrolled again |
| an enrollment code | until used or expired | a machine can join as a probe of that company and site | create another |
| `release.key` | the build machine only (`blentinelmake` key folder), **never the hub** | releases that every probe built with it installs can be signed: rotate it, and reinstall probes by hand | no new releases probes accept: create a new key and install the probes by hand once |

On Linux and macOS the hub and probe create these files readable only by their owner (mode `0600`). On Windows they inherit the folder's permissions, and by default every local user can **read** files under `C:\ProgramData` and new folders under `C:\`. **`hub install` restricts the hub's data folder** (`C:\ProgramData\Blentinel\Hub`) to SYSTEM and Administrators, inherited by everything in it (D-52); on Linux it makes `/var/lib/blentinel` `0700` for the service user and the config `0640 root:blentinel`. `hub doctor` warns if the folder is open to others. **`probe install` does the same for the probe's folder** (D-53): `C:\Program Files\Blentinel\Probe` for SYSTEM and Administrators only; on Linux `/var/lib/blentinel-probe`, `0700`, owned by the service's throwaway user (systemd `DynamicUser`). `probe doctor` warns if the folder is open to others. A probe run by hand (`probe enroll`, `./probe`) keeps its files in its own folder: restrict it yourself on a machine with other users. A hub backup (`hub backup`, the daily backup, **Back up now**, the backup before an upgrade) holds the hub's private key: it is written `0600` (owner only), in the private data folder unless `[backups] path` points elsewhere (then protect that folder too); keep it like the key. The web UI downloads one only after the user gives their password again, through a link that works once, for a minute (D-60). Files created by versions before 2026-09-27 kept the default `0644`. Folders set up by the old installer scripts already blocked other users; reinstall with `hub install` / `probe install` anyway.

None of these belong in version control. `.gitignore` excludes them.

## Reporting a vulnerability

Please report security issues privately to the maintainer rather than in a public issue.
