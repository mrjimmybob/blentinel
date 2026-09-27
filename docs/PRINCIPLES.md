# Blentinel — Founding Principles

These principles define Blentinel. **They are not negotiable.** Every proposal, pull request, and design decision must pass the principle check at the bottom of this file. If a feature cannot be built within these rules, the feature changes, not the rules.

---

## Non-negotiable (founding) principles

### FP-1 · Push-only communication

- The **probe initiates every connection**. The hub **never** opens a connection to a probe, under any circumstances.
- Probes need **no inbound ports, no port forwarding, no VPN**, and no reachable address. A probe behind NAT, CGNAT, or a strict firewall must work as long as it can make outbound HTTPS requests to the hub.
- Anything the hub wants a probe to do is delivered **in the response to a request the probe made**. This covers configuration, commands, and upgrades. It can go in the reply to a report or through an optional long-poll that the probe opens. Latency for hub→probe instructions is therefore bounded by the probe's check-in interval, and that is accepted by design.
- The hub *may* push to **browsers and mobile apps** (e.g. Server-Sent Events). FP-1 is about the hub<->probe relationship only.

### FP-2 · Zero-dependency probe

The probe is a **single, self-contained executable** per platform.

- No runtime to install (.NET, Java, Python, Node, …), no installer prerequisites, no DLLs or shared libraries shipped beside it.
- On Linux it is **statically linked (musl)**, so it runs on any distribution of the right CPU architecture.
- It uses **no external tools**. Checks are implemented natively in Rust. It does not shell out to `ping`, `snmpget`, `systemctl`, PowerShell, etc. (Explicit, opt-in, user-authored script checks are the only exception. They are off by default; see proposal 0006.)
- It uses **no OS certificate store and no system TLS library**. Trust comes from pinning (rustls).
- All of its state (identity, cached config, offline queue, logs) lives **in its own directory**.
- Removing the probe means stopping the service and deleting its directory. Nothing else is left behind.

### FP-3 · Zero-trust by default

- Every probe has its own cryptographic identity (Ed25519). The hub rejects unknown or revoked probes.
- Every report is **signed and encrypted end-to-end** (Ed25519 + X25519 + ChaCha20-Poly1305), independently of TLS.
- The probe **pins** the hub's identity: its key and TLS certificate fingerprint. It never trusts a hub just because a public CA vouched for it.
- The hub is the **authority** for which company or site a probe belongs to. Probe-supplied claims are never trusted for authorization.

### FP-4 · Resilient probe

- If the hub is unreachable, the probe **keeps checking and buffers reports locally**, then delivers them in order when the hub is reachable again.
- The buffer holds **only undelivered reports** and is **bounded** (age and count limits in `common::protocol` / `probe/src/outbox.rs`). A delivered report is deleted immediately, and a report the hub permanently rejects is never buffered (D-29).
- If the hub is unreachable, the probe **keeps running on its last known good configuration**.

---

## Product principles (mandated by the project owner)

### PP-1 · Trivial to install

- **Hub:** from download to first login in **at most 5 minutes**, with one binary, one command, and no toolchain.
- **Probe:** installed and reporting with **one command**. No file editing, no key copy-pasting, no rebuilding.
- End users never need a Rust toolchain. Building from source is for contributors.

### PP-2 · Trivial to manage

- Routine operations are done **from the web UI**, never by editing files on a server or client machine. This covers adding companies, sites, probes, checks, users, alert channels, and silences, and upgrading probes.
- Config files hold only **bootstrap settings**: where to listen, where data lives, TLS files.

### PP-3 · Lightweight and self-hosted: embedded by default, external optional

- The hub **works out of the box with no external services**: no database server, message broker, or cloud account. It uses an **embedded database (SQLite)** by default, and that default must always stay supported.
- External databases (PostgreSQL first; MySQL/MariaDB, SQL Server, and Oracle later) are **optional backends** an admin can choose. They are **never required**. Storage is modular: business logic depends only on storage traits (proposal 0011).
- Running on a small VPS or a Raspberry Pi must remain possible.

> Confirmed by the owner on 2026-09-27, with this wording ("not set in stone"; revisit via `DECISIONS.md`). See D-22.

### PP-4 · Documentation is part of the product

- A change is **not done** until the README, installation guides, and reference docs reflect it (see `ENGINEERING.md` → Definition of Done).

---

## Principle check (copy into every proposal / PR description)

| Question | Answer |
|---|---|
| FP-1: Does anything require the hub to initiate a connection to a probe? | must be **No** |
| FP-2: Does the probe gain a runtime/library/tool dependency, or a file outside its directory? | must be **No** |
| FP-3: Does any trust decision rely on unauthenticated or probe-supplied data? | must be **No** |
| FP-4: Does the probe still work (check + buffer) with the hub offline? | must be **Yes** |
| PP-1/PP-2: Does this add a manual step (file edit, key copy, rebuild) to install or manage? | should be **No** |
| PP-3: Does the hub gain a required external service? | must be **No** |
| PP-4: Are docs updated in the same change? | must be **Yes** |
