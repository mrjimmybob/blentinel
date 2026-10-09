# Probe–hub protocol

Current version: **8** (`common::protocol::PROTOCOL_VERSION`). The hub accepts versions 7 and 8 (current and previous): v8 only appends an optional field, so a v7 report is still valid, and a v7 probe updates itself to v8. Anything older is refused with `unsupported_protocol_version`. Constants both sides must agree on live in `common/src/protocol.rs`.

| Version | What changed |
|---|---|
| 2 | Reports carry a signed `protocol_version`; replay protection (D-28) |
| 3 | The hub answers with a signed, sealed `ReportAck` (D-48) |
| 4 | Results of hub-managed checks, keyed by check ID, with several metrics each; the answer carries the probe's checks (D-49) |
| 5 | Reports carry the probe's version and its last failed update; the answer may offer a signed release (D-55) |
| 6 | The answer may carry commands; reports carry their results (D-56) |
| 7 | Reports say which hub pins the probe holds; the answer may announce the hub's next pins, or that it switched to them. The TLS pin is the certificate's public key (D-61) |
| 8 | Reports carry the probe's health: when it started, the reports it holds back, its clock against the hub's, its last problem (D-73) |

## Exchange

All requests go from the probe to the hub:

| Request | Purpose |
|---|---|
| `POST /api/enroll` | Redeems an enrollment code, once (`probe enroll`). Sealed like a report. |
| `POST /api/report` | Delivers one sealed report (`Content-Type: application/octet-stream`). |
| `POST /api/probe/commands` | Asks for commands between reports; the hub may hold it open until one is queued ([below](#asking-for-commands-between-reports)). Sealed and signed like a report. |
| `GET /download/releases/{version}/{file}` | Downloads the binary of a release the hub offered (self-update, below). Not sealed: the probe checks it against the signed manifest. |

There is no key exchange: the enrollment code carries the hub's address, its X25519 public key, its signing key and its certificate's public-key pin (below). The hub changes them later by [rotation](#rotation-rotation).

Every response to these carries the header `X-Blentinel-Protocol: <hub version>`. Probes send the same header with their own version (informational; the authoritative value is signed inside the report).

## The report

Before sealing, a report is JSON:

```json
{
  "protocol_version": 8,
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
  "pins": { "current": "…64 hex…" },
  "health": {
    "started_at": "2026-03-10T09:12:44Z",
    "queued_reports": 0,
    "clock_offset_secs": 1,
    "last_error": { "at": "2026-03-10T11:02:10Z", "message": "Can't reach the hub: connection refused" }
  },
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
| `command_results[]` | v6: `{"id", "ok", "message"}` for each command the probe ran since its last report (at most 10; `message` at most 500 characters). Absent when empty. Results for commands that aren't this probe's, or are already done, are ignored. |
| `pins` | v7: `{"current", "next"?}`: digests of the hub pins the probe holds now, and of the next ones the hub announced ([rotation](#rotation-rotation)). Absent from a v6 probe. |
| `health` | v8: how the probe itself is doing (D-73). `started_at`: when this run of the probe started. `queued_reports`: reports waiting for the hub when this one was made (a backlog it then delivers in order counts until the next report). `clock_offset_secs`: the probe's clock minus the hub's, measured from the `hub_time` of the hub's last answer; absent before the first answer. `last_error`: `{"at", "message"}`, the probe's latest problem of the past 24 hours (a report it couldn't deliver or the hub refused, a check that couldn't run, keys or an enrollment it couldn't read or save); absent when there was none. Absent from a v7 probe: the hub keeps the last health it had. |
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
version (1, currently 3) | flags (1: bit 0 = has a certificate pin) | URL length (u16 BE) | hub URL (UTF-8)
| hub X25519 public key (32) | hub Ed25519 signing key (32) | [SHA-256 of the certificate's public key, SPKI DER (32)]
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

Since v5 (M2-15, D-55), when a probe release is meant for a probe that reports **another `probe_version`**, the answer carries it, unless that's the version the probe reported in `update_failed`. Which release, and when, the hub decides (M4-01, D-74): the one rolled out (to canary probes first), or the one its company is pinned to, only inside its company's update window, and none while the rollout is paused or the company is off. The probe needs no rules of its own for that.

```json
"update": {
  "manifest": "{\"format\": 1, \"version\": \"0.2.0\", …}",
  "signature": "…128 hex…",
  "rollback": { "rollback": "{\"format\": 1, \"version\": \"0.2.0\", \"from\": [\"0.3.0\"], …}", "signature": "…" }
}
```

`rollback` is there only if the release has a signed rollback (D-74), and left out otherwise; a probe that doesn't know it ignores it.

`manifest` is the text of the release's `manifest.json` exactly as signed, and `signature` the Ed25519 signature of it by a **release key**, not the hub's key:

```json
{
  "format": 1,
  "version": "0.2.0",
  "released": "2026-10-04T10:00:00Z",
  "protocol": 6,
  "artifacts": [
    { "target": "x86_64-unknown-linux-musl", "file": "probe-x86_64-unknown-linux-musl", "sha256": "…", "size": 7792504 },
    { "target": "x86_64-pc-windows-msvc", "file": "probe-x86_64-pc-windows-msvc.exe", "sha256": "…", "size": 8123392 }
  ]
}
```

The probe installs it only if all of these hold:

1. updates aren't turned off on the machine (`probe updates off`);
2. the signature verifies against a release key compiled into the probe (so the hub can't make one up);
3. `version` is newer than the probe's own (no downgrades, which also stops an old release being replayed), **or** `rollback` is signed by a release key too, is for this `version`, and lists the probe's own version in `from`. The rollback's signature covers `blentinel probe rollback v1\n` followed by its text, so a signature over a manifest can never pass for one over a rollback;
4. the release has a binary for the probe's OS and architecture;
5. the binary, downloaded from `GET /download/releases/{version}/{file}` (at most `size` bytes), has the signed SHA-256;
6. the binary passes `--self-test`.

It then switches to it and restarts, and must deliver a report within 15 minutes (or 3 intervals) and in at most 3 starts, or it goes back to the previous version. A refusal or a rollback is reported in `update_failed`, and the hub stops offering that version. File names are always `probe-<target>` (`.exe` on Windows), so a manifest can't name another file. Details: [D-55](../DECISIONS.md).

### Commands (`commands`)

Since v6 (M2-06, D-56), the answer carries the probe's pending commands, oldest first, at most 10:

```json
"commands": [
  { "id": "9b2e…c4", "kind": "run_checks" }
]
```

| Kind | What the probe does | Result |
|---|---|---|
| `run_checks` | Runs its checks at once and sends the next report straight away, instead of an interval later | `ran 3 checks` |

- A user queues a command on the hub (the probe page, or `POST /api/probes/{id}/commands`). The hub never connects to the probe: the command reaches it through the probe's **command poll** (below), usually within a second or two, or else with its next report, at most one interval later.
- The hub sends a command **in every answer** until the probe reports its result (`command_results`, above) or it expires: 3 report intervals, at least 15 minutes. So a lost answer doesn't lose the command.
- The probe runs each command once: it remembers the IDs it has run and skips them when they come again. A probe that restarts before reporting the result may run one twice, so every command is harmless to repeat (**at least once**).
- A kind the probe doesn't know (from a newer hub) doesn't make the answer invalid: the probe reports it as failed (`doesn't know this command: update it`).
- Commands are covered by the answer's signature and sealed for the probe, like its checks: only the hub can give a probe a command.

### Asking for commands between reports

Since M4-03 (D-76), a probe also asks the hub for its commands on a loop of its own, so a command doesn't wait for the next report. The report cycle doesn't change, and the protocol stays at version 8: a hub that doesn't know the request answers 404, and the probe then asks only once an hour.

The probe posts to `POST /api/probe/commands` a `CommandPoll`, sealed for the hub like a report:

```json
{ "protocol_version": 8, "probe_id": "<hex verifying key>", "timestamp": "2026-10-09T12:00:00Z", "signature": [ … ] }
```

The signature (by the probe's key) covers `blentinel command poll v1\n` followed by the JSON of the other three fields, so it can never pass for a report's. The hub refuses a poll more than 5 minutes from its own clock (`422 timestamp_rejected`), from a probe that isn't enrolled or is revoked (`403`), or that it can't open or check (`401`, `400`, `403`).

The hub **holds** the poll while no command waits, up to 25 s (under the 30 s after which many proxies drop a quiet connection), and answers as soon as one is queued for that probe; a newer poll from the same probe releases the one before. The answer is signed by the hub and sealed for the probe like the answer to a report (the signature covers `blentinel command poll answer v1\n` and the JSON), and bound to the poll by its SHA-256:

```json
{ "poll_sha256": "…", "hub_time": "…", "commands": [ { "id": "9b2e…c4", "kind": "run_checks" } ], "next_poll_secs": 0 }
```

**The hub sets the pace** with `next_poll_secs`: `0` ask again at once (after a held poll), `n` ask again in `n` seconds (the hub answers at once and says 30 when it holds 10,000 polls already), `null` don't for now (the probe asks again in an hour). The probe waits at least a second between polls whatever the answer says. When a poll fails (no answer within 45 s, an error, an answer it can't trust) the probe asks again in 30 s; a `404`, `405` or `403` makes it wait an hour. Commands it gets are run as if they came with a report, and the probe reports at once with their results. The same commands still come with every report until their results arrive, so a failing poll only makes them slower.

### Rotation (`rotation`)

Since v7 (M2-13, D-61), the hub can change what probes pin of it: its **TLS certificate** (a new key pair) and/or its **key** (X25519, and the Ed25519 signing key derived from it). The probe pins the SHA-256 of the certificate's **public key** (SPKI), not of the whole certificate, so a certificate renewed with the same key needs no rotation.

A probe's pins are summed up as a **digest**: SHA-256 (hex) of the hub key (32), the signing key (32), then `0x01` and the certificate pin (32), or `0x00` without one (`common::pins::HubPins::digest`). Reports carry the digests of what the probe holds (`pins`, above).

1. **Announce.** While a rotation is in progress, the answer to a v7 report whose `pins` don't include the next ones carries them:

   ```json
   "rotation": {
     "pins": { "hub_key": "…64 hex…", "signing_key": "…64 hex…", "tls_spki_sha256": "…64 hex…" },
     "switched": false
   }
   ```

   The answer is signed with the **current** key, which the probe already trusts, so nobody else can plant pins. The probe saves them in `enrollment.json` (`next`) beside its current ones, accepts either certificate and either signing key from then on, and reports both digests. A probe that doesn't pin a certificate (plain HTTP, or a proxy) keeps not pinning one.
2. **Switch.** When every active probe reports the next pins, the hub switches by itself; an admin can switch earlier. The hub serves the new certificate in place (no restart) and signs with the new key. It keeps the previous key, so reports a probe sealed before it switched still open, until no probe can still hold one (the buffer's age limit plus clock skew, 7 days 5 minutes).
3. **Told.** The answer to a probe whose current pins aren't the hub's carries the hub's pins with `"switched": true`. The probe, which trusts the answer (it holds the new signing key), makes them its only pins and seals its next report for the new key. Its buffered reports stay (the hub can still open them).

A probe that didn't take the next pins before the switch can't trust the hub's answers (new key) or reach it (new certificate): it must be enrolled again. Enrollment codes made before a switch carry the old pins, so the switch revokes them.
