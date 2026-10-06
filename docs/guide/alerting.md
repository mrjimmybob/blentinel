# Alerting

The hub can e-mail alerts when something breaks and when it recovers. Configure it in the `[alerts]` section ([reference/configuration.md](../reference/configuration.md#alerts)). Alerts are off until `enabled = true` and SMTP is set up.

## When you get an alert

| Alert | Sent when | Then |
|---|---|---|
| **DOWN** | a check is Down in **2 results in a row** (also when it's down from the start) | nothing more while it stays down |
| **RECOVERED** | a check that was down is Up in 2 results in a row | |
| **Threshold** (`disk`/`cpu`/`memory` N%) | a `disk`, `cpu` or `memory` check's usage is at or over its threshold (defaults: disk 90 %, CPU 95 %, memory 90 %) in 2 reports in a row | nothing more while it stays over; once it is back under for 2 reports, the next breach alerts again (no e-mail for that) |
| **PROBE EXPIRED** | a probe hasn't reported for `probe_timeout_secs` | nothing more for this outage; the next outage alerts again |

Alerts go to `default_recipients`, or to the company's `alert_emails` if `[alerts.company_overrides."<company>"]` exists. That override can also set different thresholds.

## Several results in a row (flap suppression)

A check alerts only when **several results in a row** agree (DECISIONS D-63): by default 2 to go down, 2 to recover. One lost ping, one slow page or one CPU spike alerts nobody, and a link that comes and goes doesn't send DOWN and RECOVERED every other minute. A real outage is reported one probe interval later than the first failure (with the default 60 s interval: about 2 minutes after it starts).

Each check can say otherwise: on the probe's page, **Edit** the check and fill **Alert after _N_ failed results in a row; recovered after _M_ good ones** (1 to 10; empty: the default, 2). 1 alerts on the very first failure, as before. A higher number suits checks that fail now and then on their own (a ping over a busy Wi-Fi link); count it in reports: with a 5-minute interval, 3 failures is a quarter of an hour. The same numbers apply to the check's usage threshold. They are hub settings only: changing them doesn't send the probe a new config.

The results are counted in the order the probe made them, also when it delivers a batch it buffered while the hub was unreachable.

Alert state belongs to the check (its ID, D-49): renaming a check or changing its target keeps it, and a new check starts fresh.

## Silencing a check

From the probe's device list in the UI (or `POST /api/silence`), silence a check for 1 hour, 24 hours, 7 days, or until further notice. While it's silenced:

- it sends no alerts (DOWN, RECOVERED or threshold);
- its state is still tracked (results in a row included), so nothing fires unexpectedly when the silence ends.

A silence ends when:

- its time runs out;
- you remove it (UI or `POST /api/silence/delete`);
- or **the check recovers** (its good results in a row, as above). The silence was for that outage. No RECOVERED alert is sent for an outage nobody was alerted about.

Deleting a check deletes its silence. Only individual checks can be silenced today. Silencing a whole probe, site or company, and scheduled maintenance windows, are planned (proposal 0005).

## Testing your setup

1. Enable alerts with a recipient you can read.
2. Give a probe a check that fails, e.g. a `tcp` check on a closed port ([checks.md](../reference/checks.md)).
3. Within two probe intervals you should get a DOWN e-mail (set the check's **Alert after** to 1 to get it after one). SMTP errors appear in the hub's log as `ERROR Failed to send … alert: …`.

## Coming later

Teams/Slack/webhook channels, incidents with acknowledgement, dependencies ("don't alert for everything behind a dead router"), and escalation: see [proposal 0005](../proposals/0005-alerting-v2.md). The code is ready for new channels: alerts go through a `Notifier` interface (`hub/src/service/alerts.rs`), and e-mail is one implementation.
