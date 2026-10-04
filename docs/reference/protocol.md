# Probe–hub protocol

Current version: **5** (`common::protocol::PROTOCOL_VERSION`). The hub accepts versions 4 and 5 (current and previous): v5 only appends optional fields, so a v4 report is still valid. v4 changed what a report is (results of hub-managed checks, D-49), so anything older is refused with `unsupported_protocol_version`. Constants both sides must agree on live in `common/src/protocol.rs`.

| Version | What changed |
|---|---|
| 2 | Reports carry a signed `protocol_version`; replay protection (D-28) |
| 3 | The hub answers with a signed, sealed `ReportAck` (D-48) |
| 4 | Results of hub-managed checks, keyed by check ID, with several metrics each; the answer carries the probe's checks (D-49) |
| 5 | Reports carry the probe's version and its last failed update; the answer may offer a signed release (D-55) |

## Exchange

All requests go from the probe to the hub:

| Request | Purpose |
|---|---|
| `POST /api/enroll` | Redeems an enrollment code, once (`probe enroll`). Sealed like a report. |
| `POST /api/report` | Delivers one sealed report (`Content-Type: application/octet-stream`). |
| `GET /download/releases/{version}/{file}` | Downloads the binary of a release the hub offered (self-update, below). Not sealed: the probe checks it against the signed manifest. |

There is no key exchange: the enrollment code carries the hub's address, its X25519 public key and its certificate fingerprint (below).

Every response to these carries the header `X-Blentinel-Protocol: <hub version>`. Probes send the same header with their own version (informational; the authoritative value is signed inside the report).

## The report

Before sealing, a report is JSON:

```json
{
  "protocol_version": 5,
  "probe_id": "a773d2…dfcd",
  "company_id": "example-company",
  "hostname": "office-pc-01",
  "site": "head-office",
  "timestamp": "2026-03-10T12:00:00.123456Z",
  "interval_seconds": 60,
  "results": [
    { "check_id": "5f0c…a1", "status": "Up", "message": "Responded in 2ms", "latency_ms": 2 },
    {
      "check_id": "9b77…3e", "status": "Up", "message": "/: 41.0 GB / 98.0 GB (41.8%)",
      "metrics": [
        { "key": "usage", "value": 41.8, "unit": "%" },
        { "key": "used", "value": 41.0, "unit": "GB" },
        { "key": "free", "value": 57.0, "unit": "GB" },
        { "key": "total", "value": 98.0, "unit": "GB" }
      ]
    }
  ],
  "config_revision": "3f1a9c0e5b7d2468",
  "probe_version": "0.2.0",
  "signature": [ /* 64 bytes */ ]
}
```

| Field | Notes |
|---|---|
| `protocol_version` | Read first: a report of another version is refused for its version, not as malformed |
| `probe_id` | Hex of the probe's Ed25519 public key |
| `company_id`, `site` | The probe's own view, for its logs; the hub files the report by the probe's record |
| `timestamp` | When the checks ran (probe clock, UTC). Used for replay protection. |
| `results[]` | One per check the probe runs: `check_id` (the hub's), `status` (`Up` or `Down`), `message`, `latency_ms` and `metrics` when there are any. The metrics each kind reports are in [checks.md](checks.md). Results for checks that aren't this probe's are ignored. |
| `config_revision` | The revision of the checks the probe runs; absent before it has any |
| `config_rejected` | `{"revision", "error"}`: the last config the hub sent that the probe couldn't use |
| `probe_version` | v5: the probe's version. Absent from a v4 probe. |
| `update_failed` | v5: `{"version", "error"}`: the last update the probe refused or rolled back, until it installs another version |
| `signature` | Ed25519 over the canonical *signable* report (every field above except `signature`, in this order, absent optional fields left out). **Required.** |

**Changing the format:** field order is part of what's signed. New fields are appended and omitted when absent. A unit test in `common/src/models.rs` pins the signed bytes.

## The sealed payload

```
bytes 0–31   probe's ephemeral X25519 public key (new for every report)
bytes 32–43  ChaCha20-Poly1305 nonce (random)
bytes 44–    ciphertext of the report JSON + 16-byte authentication tag
```

The key is the X25519 shared secret between the ephemeral key and the hub's long-term key. See [security.md](../security.md).

## Enrollment

The code (`common::enrollment::EnrollmentCode`) is `BLT1-` followed by the Crockford base32 encoding (case-insensitive; `O`→`0`, `I`/`L`→`1`; `-` and spaces ignored) of:

```
version (1, currently 2) | flags (1: bit 0 = has certificate fingerprint) | URL length (u16 BE) | hub URL (UTF-8)
| hub X25519 public key (32) | hub Ed25519 signing key (32) | [SHA-256 of the hub's certificate, DER (32)]
| token ID (8) | secret (16) | CRC-32 of everything before (4, BE)
```

`probe enroll` sends `POST /api/enroll`, sealed exactly like a report, with this JSON (`SignedEnrollRequest`):

```json
{
  "request": {
    "token_id": "9f86d081884c7d65", "secret": "…32 hex…",
    "probe_id": "a773d2…dfcd", "hostname": "frontdesk-pc", "os": "windows", "arch": "x86_64",
    "probe_version": "0.1.0", "timestamp": "2026-09-28T12:00:00Z"
  },
  "signature": "…128 hex: Ed25519 over the JSON of `request`, by the key `probe_id` names…"
}
```

| Status | Error code | Meaning |
|---|---|---|
| **200** | | Enrolled: `{"probe_id", "name", "company_id", "company_name", "site_id", "site_name"}` |
| **400** | `invalid_body` | Not an enrollment request |
| **401** | `undecryptable` | Not sealed for this hub's key |
| **403** | `bad_signature`, `invalid_code` | Not signed by the probe it names; or the code is unknown, wrong, expired, used up or revoked |
| **422** | `timestamp_rejected` | The probe's clock is more than 5 minutes off |
| **429** | `too_many_attempts` | Too many failed codes from this address (the login limits) |

Enrolling a probe ID that already has a record replaces it (new company, site and name; active again). The report fields `company_id` and `site` are kept for the probe's own logs; **the hub ignores them** and stores each report under the company and site of the probe's record.

## Responses to `POST /api/report`

| Status | Error code | Meaning | What the probe does |
|---|---|---|---|
| **200** | | Stored. The body is the hub's [answer](#the-hubs-answer) | Deletes it from its buffer |
| **409** | `not_newer` | Already have this or a newer report from this probe (replay, or duplicate delivery) | Deletes it: nothing to resend |
| **400** | `not_a_report`, `unsupported_protocol_version` | Not a report, or unsupported protocol version | Drops it (permanent) |
| **401** | `undecryptable` | Can't be opened: sealed for another hub key, or corrupted | Drops it (permanent) |
| **403** | `unknown_probe`, `missing_signature`, `bad_signature` | Probe not enrolled; or a missing or invalid signature | Drops it (permanent) |
| **403** | (`probe_revoked`) | The probe was revoked. The body is the hub's [answer](#the-hubs-answer) with `"revoked": true` | Stops reporting, clears its buffer, moves `enrollment.json` to `enrollment.revoked.json` and waits to be enrolled again |
| **422** | `timestamp_rejected` | Timestamp outside the window: > 5 min in the future (probe clock ahead?) or > 7 days old | Drops it (permanent) |
| **5xx**, 408, 429, network error | `internal` (500) | Hub-side problem | Keeps it and retries |

The other error responses have the same JSON body as the rest of the API, `{"error": {"code": "unknown_probe", "message": "report from unregistered probe a773d201 (enroll it with a code)"}}` (`common::protocol::ErrorBody`). The probe decides by status code alone, and logs the message and code when it drops a report, e.g. `Hub answered 403 Forbidden: report from unregistered probe a773d201 (enroll it with a code) (unknown_probe)`.

The hub checks in this order: open → version → parse → signature (by the key the report names) → enrolled → not revoked → only results of the probe's own checks → time window → newer than the latest stored. The time checks come after the signature, so forged traffic learns nothing about them.

## The hub's answer

Since v3 (M2-03, D-48), the hub answers a report whose signature checked out with a **`ReportAck`** (`common::ack`), as the body of `200` and of `403 probe_revoked` (`Content-Type: application/octet-stream`):

```
sealed for the probe ( hub Ed25519 signature (64) | ack JSON )
```

```json
{
  "report_sha256": "…64 hex: SHA-256 of the sealed report it answers…",
  "hub_time": "2026-09-29T10:00:00Z",
  "revoked": false,
  "config": {
    "revision": "3f1a9c0e5b7d2468",
    "interval_secs": 60,
    "checks": [
      { "id": "5f0c…a1", "name": "Core switch", "spec": { "kind": "ping", "host": "192.0.2.1" } }
    ]
  }
}
```

- **Signed** with the hub's signing key, whose public half is in the enrollment code. The signature covers the JSON bytes as sent, so a newer hub may add fields and an older probe ignores them.
- **Sealed** for the probe with the same format as a report, to the X25519 form of the probe's Ed25519 identity key (the birational map; `common::crypto::seal_for_probe`).
- **Bound** to one report by `report_sha256`, so an answer can't be replayed for another report.

The probe trusts only an answer that opens with its key, carries the hub's signature and names the report it sent. An answer that fails these checks is logged (`its answer can't be trusted`) and ignored; the report still counts as delivered on a `200`. A `403` without a trusted answer only drops the report, because anyone on the network path can send a plain `403`.

`hub_time` lets the probe warn when its clock is more than a minute off the hub's, before the hub starts refusing reports (at 5 minutes).

### The probe's checks (`config`)

Since v4 (M2-04, D-49), the answer to a stored report carries `config`, everything the probe should run (its enabled checks, by ID, and its report interval), **when the report's `config_revision` is a different one**, and it isn't the revision the probe last refused. The `revision` is the first 16 hex digits of a SHA-256 of the rest of the config, so it changes with any change on the hub and needs no counter.

- The config is covered by the answer's signature, so only the hub can give a probe checks, even over plain HTTP.
- The probe checks it with the same validation as the hub (`ProbeConfig::validate`) and applies it **whole or not at all**. It saves it to `config.json` (atomically), runs it at once instead of an interval later, and reports its revision from then on.
- A config it can't use (for example a check kind this probe version doesn't know) is refused: the probe keeps running the previous one and reports `config_rejected` until the hub sends another; the hub shows the error and doesn't send that revision again.
- If the hub is unreachable, the probe runs its last config (FP-4).

### Self-update (`update`)

Since v5 (M2-15, D-55), when an admin has approved a probe release on the hub, the answer to a probe that reports **another `probe_version`** carries it, unless that's the version the probe reported in `update_failed`:

```json
"update": {
  "manifest": "{\"format\": 1, \"version\": \"0.2.0\", …}",
  "signature": "…128 hex…"
}
```

`manifest` is the text of the release's `manifest.json` exactly as signed, and `signature` the Ed25519 signature of it by a **release key**, not the hub's key:

```json
{
  "format": 1,
  "version": "0.2.0",
  "released": "2026-10-04T10:00:00Z",
  "protocol": 5,
  "artifacts": [
    { "target": "x86_64-unknown-linux-musl", "file": "probe-x86_64-unknown-linux-musl", "sha256": "…", "size": 7792504 },
    { "target": "x86_64-pc-windows-msvc", "file": "probe-x86_64-pc-windows-msvc.exe", "sha256": "…", "size": 8123392 }
  ]
}
```

The probe installs it only if all of these hold:

1. updates aren't turned off on the machine (`probe updates off`);
2. the signature verifies against a release key compiled into the probe (so the hub can't make one up);
3. `version` is newer than the probe's own (no downgrades, which also stops an old release being replayed);
4. the release has a binary for the probe's OS and architecture;
5. the binary, downloaded from `GET /download/releases/{version}/{file}` (at most `size` bytes), has the signed SHA-256;
6. the binary passes `--self-test`.

It then switches to it and restarts, and must deliver a report within 15 minutes (or 3 intervals) and in at most 3 starts, or it goes back to the previous version. A refusal or a rollback is reported in `update_failed`, and the hub stops offering that version. File names are always `probe-<target>` (`.exe` on Windows), so a manifest can't name another file. Details: [D-55](../DECISIONS.md).

Later items add commands (M2-06) and key rotation (M2-13) to the answer.
