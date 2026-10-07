# Alerting

The hub can e-mail alerts when something breaks and when it recovers. Configure it in the `[alerts]` section ([reference/configuration.md](../reference/configuration.md#alerts)). Alerts are off until `enabled = true` and SMTP is set up.

## When you get an alert

| Alert | Sent when | Then |
|---|---|---|
| **DOWN** | a check is Down in **2 results in a row** (also when it's down from the start): it failed, or is over a down threshold | nothing more while it stays down |
| **WARNING** | a check is over a warning threshold in 2 results in a row (or was down and is now only over its warning limit) | nothing more while it stays in warning |
| **RECOVERED** | a check that was down or in warning is Up in 2 results in a row | |
| **PROBE EXPIRED** | a probe hasn't reported for `probe_timeout_secs` | nothing more for this outage; the next outage alerts again |

Alerts go to `default_recipients`, or to the company's `alert_emails` if `[alerts.company_overrides."<company>"]` exists. The e-mail says why: the probe's message, or the threshold, e.g. `disk usage 93% is over the warning limit (90%, from the company)`.

## Statuses

The hub gives each check result one of four statuses (DECISIONS D-16, D-64), the same on the dashboard, the probe page, in uptime and in alerts:

| Status | When |
|---|---|
| **Up** | it worked, and is under its thresholds |
| **Warning** | it works, but is at or over a **warning** limit: look at it. Counts as up in uptime |
| **Down** | it failed (the probe says so), or is at or over a **down** limit |
| **Unknown** | its probe stopped reporting: the last result is history. Its probe alerts once (PROBE EXPIRED), not each check |

A company or probe with a check in Warning shows amber on the dashboard; one whose probe stopped reporting shows grey (degraded).

## Thresholds

Limits apply to the **usage** of `cpu`, `memory` and `disk` checks (percent) and the **response time** of `ping`, `http` and `tcp` checks (milliseconds). Each has a warning limit and a down limit; at or over one, the check is Warning or Down.

Each limit comes from the first of these that sets it:

1. **the check** (its dialog on the probe's page: *warning at / down at*);
2. **its probe's site** (the company page, **Thresholds**, *For: Site …*);
3. **the company** (the company page, **Thresholds**, *For: The whole company*);
4. **the hub's default** (the admin page, **Thresholds**);
5. **the built-in default**:

| | Warning at | Down at |
|---|---|---|
| CPU | 95 % | off |
| Memory | 90 % | off |
| Disk | 90 % | 97 % |
| Latency | off | off |

So a company can be stricter about disks, one site with a slow link can allow 500 ms where the rest warn at 200 ms, and one check can switch a limit **off** (`off`) that the levels above set. An empty limit is inherited: the form shows what applies then as a grey placeholder, and where it comes from when you point at it. Changes apply from each probe's next report (results already stored keep the status they had). Technicians set a company's and its sites'; Owners and Admins the hub's. Changes are in the audit log (`thresholds.update`).

## Several results in a row (flap suppression)

A check alerts only when **several results in a row** agree (DECISIONS D-63): by default 2 to go down, 2 to recover. One lost ping, one slow page or one CPU spike alerts nobody, and a link that comes and goes doesn't send DOWN and RECOVERED every other minute. A real outage is reported one probe interval later than the first failure (with the default 60 s interval: about 2 minutes after it starts).

Each check can say otherwise: on the probe's page, **Edit** the check and fill **Alert after _N_ failed results in a row; recovered after _M_ good ones** (1 to 10; empty: the default, 2). 1 alerts on the very first failure, as before. A higher number suits checks that fail now and then on their own (a ping over a busy Wi-Fi link); count it in reports: with a 5-minute interval, 3 failures is a quarter of an hour. Warning counts like Down for going in (failed results), like a recovery for going back to Up. They are hub settings only: changing them doesn't send the probe a new config.

The results are counted in the order the probe made them, also when it delivers a batch it buffered while the hub was unreachable.

Alert state belongs to the check (its ID, D-49): renaming a check or changing its target keeps it, and a new check starts fresh.

## Maintenance windows

Planned work (patching, a router swap, a move) shouldn't page anyone. A **maintenance window** says when, ahead of time, once or on a schedule (DECISIONS D-65). On the company page, follow **Maintenance and silences**:

- **What it covers:** the whole company, a site, a probe or one check.
- **When:** once (from a date and time to another), every week (chosen days, a start time, a length), or every month (a day of the month, or the 1st–4th or last Monday … Sunday: *the 2nd Tuesday* is Patch Tuesday). Up to 7 days long.
- **Time zone:** its times are local to a time zone, prefilled with the site's (set on the company page, in the site's row; `UTC` by default), so 22:00 stays 22:00 there when the clocks change. A start in the hour the clocks skip opens when that hour ends; one in the hour they repeat, the first time round.

While a window is open:

- what it covers **sends no alerts** (DOWN, WARNING, RECOVERED, and PROBE EXPIRED for its probes);
- statuses are still recorded and shown: the device list marks results that came during maintenance;
- its results **don't count for uptime**; the uptime chart shades those times.

When it closes, **whatever is still wrong alerts** with the next report: a server that didn't come back after its reboot isn't missed. A probe that stopped reporting during the window alerts once it has closed, if it is still silent. What went wrong and recovered within the window alerts nobody.

The list shows each window, what it covers, its schedule, and when it next opens (or "Open now"). Removing one takes effect at once. Technicians manage windows of their companies; Viewers and Clients see them. Changes are in the audit log (`maintenance.create`, `maintenance.delete`).

## Silences

A silence is the unplanned kind: from now, for 1 hour, 24 hours, 7 days, or until removed. Silence one check from the probe's device list, or a probe, a site or the whole company on the maintenance page (or `POST /api/silence`). While something is silenced:

- it sends no alerts (DOWN, WARNING or RECOVERED; a silenced probe, site or company holds back PROBE EXPIRED too, until the silence ends);
- its state is still tracked (results in a row included), so nothing fires unexpectedly when the silence ends.

A silence ends when:

- its time runs out;
- you remove it (UI or `POST /api/silence/delete`);
- or, for a check's silence, **the check recovers** (its good results in a row, as above). The silence was for that outage. No RECOVERED alert is sent for an outage nobody was alerted about.

Deleting a check deletes its silence. Unlike a window, a silence doesn't take results out of uptime.

## Testing your setup

1. Enable alerts with a recipient you can read.
2. Give a probe a check that fails, e.g. a `tcp` check on a closed port ([checks.md](../reference/checks.md)).
3. Within two probe intervals you should get a DOWN e-mail (set the check's **Alert after** to 1 to get it after one). SMTP errors appear in the hub's log as `ERROR Failed to send … alert: …`.

## Coming later

Teams/Slack/webhook channels, incidents with acknowledgement, dependencies ("don't alert for everything behind a dead router"), and escalation: see [proposal 0005](../proposals/0005-alerting-v2.md). The code is ready for new channels: alerts go through a `Notifier` interface (`hub/src/service/alerts.rs`), and e-mail is one implementation.
