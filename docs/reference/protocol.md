# Probe–hub protocol

Current version: **2** (`common::protocol::PROTOCOL_VERSION`). The hub accepts versions 1 and 2. Constants both sides must agree on live in `common/src/protocol.rs`.

## Exchange

All requests go from the probe to the hub:

| Request | Purpose |
|---|---|
| `GET /api/handshake` | Returns the hub's X25519 public key (64 hex chars). The probe calls it at startup unless `hub_public_key` is configured. |
| `POST /api/report` | Delivers one sealed report (`Content-Type: application/octet-stream`). |

Every response to these carries the header `X-Blentinel-Protocol: <hub version>`. Probes send the same header with their own version (informational; the authoritative value is signed inside the report).

## The report

Before sealing, a report is JSON:

```json
{
  "probe_id": "a773d2…dfcd",
  "company_id": "example-company",
  "hostname": "office-pc-01",
  "site": "head-office",
  "timestamp": "2026-03-10T12:00:00.123456Z",
  "interval_seconds": 60,
  "resources": [
    {
      "name": "Core switch",
      "resource_type": "ping",
      "target": "192.0.2.1",
      "status": "Up",
      "message": "Responded in 2ms",
      "latency_ms": 2,
      "metric_value": null,
      "metric_unit": null
    }
  ],
  "signature": [ /* 64 bytes */ ],
  "ephemeral_public_key": null,
  "protocol_version": 2
}
```

| Field | Notes |
|---|---|
| `probe_id` | Hex of the probe's Ed25519 public key |
| `timestamp` | When the checks ran (probe clock, UTC). Used for replay protection. |
| `resources[].resource_type` | `ping`, `http`, `tcp`, `local_disk`, `local_cpu`, `local_mem`, `local_uptime` |
| `resources[].status` | `Up` or `Down` |
| `resources[].metric_value` / `metric_unit` | For local metrics: a percentage (`%`) or uptime (`seconds`) |
| `signature` | Ed25519 over the canonical *signable* report (every field above except `signature` and `ephemeral_public_key`, in this order). **Required.** |
| `protocol_version` | Absent means version 1 |

**Changing the format:** field order is part of what's signed. New fields are appended and omitted when absent, so older reports keep exactly the bytes their probes signed. A unit test in `common/src/models.rs` pins the version-1 bytes.

## The sealed payload

```
bytes 0–31   probe's ephemeral X25519 public key (new for every report)
bytes 32–43  ChaCha20-Poly1305 nonce (random)
bytes 44–    ciphertext of the report JSON + 16-byte authentication tag
```

The key is the X25519 shared secret between the ephemeral key and the hub's long-term key. See [security.md](../security.md).

## Responses to `POST /api/report`

| Status | Meaning | What the probe does |
|---|---|---|
| **200** | Stored | Deletes it from its buffer |
| **409** | Already have this or a newer report from this probe (replay, or duplicate delivery) | Deletes it: nothing to resend |
| **400** | Not a report, or unsupported protocol version | Drops it (permanent) |
| **401** | Can't be opened: sealed for another hub key, or corrupted | Drops it (permanent) |
| **403** | Unknown probe, or missing or invalid signature | Drops it (permanent) |
| **422** | Timestamp outside the window: > 5 min in the future (probe clock ahead?) or > 7 days old | Drops it (permanent) |
| **5xx**, 408, 429, network error | Hub-side problem | Keeps it and retries |

The hub checks in this order: open → parse and version → registered probe → signature → time window → newer than the latest stored. The time checks come after the signature, so forged traffic learns nothing about them.
