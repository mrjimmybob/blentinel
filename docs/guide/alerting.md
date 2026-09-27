# Alerting

The hub can e-mail alerts when something breaks and when it recovers. Configure it in the `[alerts]` section ([reference/configuration.md](../reference/configuration.md#alerts)). Alerts are off until `enabled = true` and SMTP is set up.

## When you get an alert

| Alert | Sent when | Then |
|---|---|---|
| **DOWN** | a resource goes from Up to Down, or is Down the first time it's seen | nothing more while it stays down |
| **RECOVERED** | a resource that was Down is Up again | |
| **Threshold** (`disk`/`cpu`/`memory` N%) | a local metric reaches its threshold (defaults: disk 90 %, CPU 95 %, memory 90 %) | nothing more while it stays at or above; once it drops below, the next breach alerts again |
| **PROBE EXPIRED** | a probe hasn't reported for `probe_timeout_secs` | nothing more for this outage; the next outage alerts again |

Alerts go to `default_recipients`, or to the company's `alert_emails` if `[alerts.company_overrides."<company>"]` exists. That override can also set different thresholds.

Today every change alerts immediately. A single failed check therefore sends DOWN and then RECOVERED a minute later. Waiting for several failures in a row is planned (ROADMAP M3, proposal 0005).

## Silencing a resource

From the probe's device list in the UI (or `POST /api/silence`), silence a resource for 1 hour, 24 hours, 7 days, or until further notice. While it's silenced:

- it sends no alerts (DOWN, RECOVERED or threshold);
- its state is still tracked, so nothing fires unexpectedly when the silence ends.

A silence ends when:

- its time runs out;
- you remove it (UI or `POST /api/silence/delete`);
- or **the resource recovers**. The silence was for that outage. No RECOVERED alert is sent for an outage nobody was alerted about.

Only individual resources can be silenced today. Silencing a whole probe, site or company, and scheduled maintenance windows, are planned (proposal 0005).

## Testing your setup

1. Enable alerts with a recipient you can read.
2. Add a resource that fails, e.g. a `tcp` check on a closed port.
3. Within one probe interval you should get a DOWN e-mail. SMTP errors appear in the hub's log as `[ALERT ERROR] Failed to send …`.

## Coming later

Teams/Slack/webhook channels, incidents with acknowledgement, dependencies ("don't alert for everything behind a dead router"), and escalation: see [proposal 0005](../proposals/0005-alerting-v2.md). The code is ready for new channels: alerts go through a `Notifier` interface (`hub/src/service/alerts.rs`), and e-mail is one implementation.
