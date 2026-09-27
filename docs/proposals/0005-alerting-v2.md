# 0005 · Alerting v2 — Trustworthy, Routable Alerts

| | |
|---|---|
| **Status** | Proposed |
| **Milestone** | M3 — Trustworthy alerting |
| **Priority** | P1 (flap suppression, statuses, maintenance, channels) · P2 (incidents/ack, escalation, dependencies) · P3 (PSA) |
| **Depends on** | 0001 (users for ack/audit), 0003 (`CheckId`) |
| **Decisions** | D-16 |

## 1. Problem

- **Alerts fire on the first failed check** (`alerts/state.rs::should_send_alert` compares only against the last status). One dropped ping sends an e-mail, and the recovery e-mail follows a minute later. This kind of alert fatigue is how monitoring ends up ignored.
- Status is binary. A disk at 95% shows **Up** in the UI; the threshold breach only exists as an e-mail (D-16).
- E-mail is the only channel, and SMTP, technicians, and recipients are configured in TOML.
- Silences exist per device only (`scope_type` supports more, but only `"resource"` is used). There are no scheduled maintenance windows, so every Patch Tuesday triggers alerts.
- If a site's router dies, every device behind it alerts separately.
- There's no acknowledgement and no record of who is handling an incident.

## 2. Design

### 2.1 Status model (single source of truth, evaluated on the hub)

`Up | Warning | Down | Unknown`

- **Warning:** a threshold breach (metric or latency), or a TLS cert expiring within the warning window.
- **Down:** the check failed, or a critical threshold was breached.
- **Unknown:** the probe is expired or stale, or a parent dependency is Down (§2.5).
- Thresholds resolve in this order: **check → site → company → global default**, as warn/crit pairs. The evaluation code lives in `common`/`service` and is used by the UI, API, and alert engine alike.

### 2.2 Flap suppression (P1, the highest-value item in this proposal)

Per check, with defaults set globally:

- `fail_after`: consecutive non-Up results before the state becomes Down or Warning (default **2**).
- `recover_after`: consecutive Up results before the state becomes Up (default **2**).
- Optional `flap_detection`: if the state changes more than K times in a window, mark it **Flapping**, send one alert, and suppress further alerts until it's stable.

This is implemented as a **pure state machine** (`fn next(state, result, policy) -> (state, Vec<Event>)`) and exhaustively unit-tested.

### 2.3 Incidents and acknowledgement (P2)

```sql
incidents (id, check_id, probe_id, company_id, severity, opened_at, closed_at,
           acknowledged_by, acknowledged_at, ack_note, assigned_to)
incident_events (incident_id, at, kind, user_id, details)   -- timeline
```

- A transition to Down or Warning opens an incident, and recovery closes it. Probe expiry opens **one** incident for the probe; its checks become Unknown without individual alerts.
- **Acknowledge** ("I'm on it", with a note) stops re-notification and shows the owner in the UI and to other techs.
- **Re-notify** every X h while unacknowledged (default off). **Escalate** to a secondary route after Y min unacknowledged (P2).

### 2.4 Maintenance windows and silences (P1)

- **Maintenance windows:** one-off or recurring (weekly/monthly rules, in the **site timezone**), scoped to company, site, probe, or check. They suppress notifications and are **excluded from SLA/uptime** calculations (0007). They show as shaded bands on charts.
- **Silences:** the existing feature, extended to company, site, and probe scope. They reference `CheckId` (D-15). Options: duration, "until recovery" (the current behaviour), or forever. Every silence records who created it and why.

### 2.5 Dependencies (P2)

- A check can declare `depends_on` (another check, usually the site gateway ping). While the parent is Down, children are **Unknown (unreachable)** and do not notify. The incident names the root cause.
- Suggested automatically: the first ping check on a probe's default gateway.

### 2.6 Notification channels and routing (P1)

Channels are configured in the UI, secrets are encrypted at rest (hub data key), and each has a **Send test** button:

| Channel | Priority |
|---|---|
| E-mail (SMTP; existing, moved to UI) | P1 |
| Generic webhook (JSON, HMAC-SHA256 signature header) | P1 |
| Microsoft Teams (workflow webhook) | P1 |
| Slack, Discord | P1 |
| ntfy, Pushover, Telegram (mobile push without an app) | P2 |
| SMS (Twilio or similar) | P3 |
| PSA ticketing: HaloPSA, ConnectWise Manage, Autotask, Syncro. Creates a ticket on incident, adds a note on ack, closes on recovery | P3 |

**Routing rules** (ordered, first match wins or continue): match on company, site, severity, check kind, or tag → one or more channels. Per-user **quiet hours** apply to personal channels. Optional **daily digest** e-mail per company (a summary of incidents, warnings, and upcoming cert expiries).

### 2.7 Notification content

- It says what failed, where (company › site › probe › check), since when, the last message, the metric value versus the threshold, and a deep link to the check page. Recovery messages include the downtime duration.
- Templates live in one module and are shared by all channels (DRY). Each channel formats the same `Notification` struct.

## 3. Principle check

All logic runs on the hub. The probe only sends raw results (FP-1, FP-2 unaffected). PP-2: everything is configured in the UI; the `[alerts]` TOML is imported once (D-04). PP-3: no external services are required. Channels are optional.

## 4. Tests

- The state machine: property-based tests (e.g. `proptest`). No alert on a single failure; exactly one Down and one Up event per real outage; flapping is suppressed.
- Threshold resolution order; maintenance-window recurrence across DST changes; dependency suppression.
- Channel adapters: payload golden files; HMAC signature verification; retry with backoff on failure. A failed channel is itself surfaced in the UI.
- Integration: probe expiry → a single incident; ack stops re-notify; silence scopes.

## 5. Docs to update

`docs/guide/alerting.md` (new: statuses, thresholds, flap settings, maintenance, channels, routing), `docs/reference/config.md` (remove `[alerts]`), CHANGELOG (behaviour change: alerts now wait for 2 consecutive failures by default).
