# Alerting

The hub sends an alert when something breaks and when it recovers, by e-mail, to a webhook of your own, to Microsoft Teams, Slack, Discord or Telegram, or to phones with ntfy. None of them is required: use the ones you have. Everything is set on the **Alerts** page (Owners and Admins; DECISIONS D-66): the **channels** alerts can go to, and the **routing rules** that say which alerts go where. A new hub has neither, so it sends nothing until you add both.

## When you get an alert

| Alert | Sent when | Then |
|---|---|---|
| **DOWN** | a check is Down in **2 results in a row** (also when it's down from the start): it failed, or is over a down threshold | nothing more while it stays down |
| **WARNING** | a check is over a warning threshold in 2 results in a row (or was down and is now only over its warning limit) | nothing more while it stays in warning |
| **RECOVERED** | a check that was down or in warning is Up in 2 results in a row | |
| **PROBE EXPIRED** | a probe hasn't reported for `probe_timeout_secs` | nothing more for this outage; the next outage alerts again |

Each alert says what (company › site › probe › check, the check's kind and target), when, and why: the probe's message, or the threshold, e.g. `disk usage 93% is over the warning limit (90%, from the company)`. A RECOVERED alert also says how long the outage lasted. With the hub's address set (below) it links to its [incident](#incidents). The subject is `[BLENTINEL #12][<company>][<site>] <check> DOWN` (WARNING, RECOVERED), or `[BLENTINEL #12][<company>] PROBE <name> EXPIRED`: `#12` is its [incident](#incidents), the same on every alert about it (see [Tickets in your PSA](#tickets-in-your-psa)). A test has `[BLENTINEL]`.

## Channels

On the **Alerts** page, **Add a channel**: give it a name and a kind.

| Kind | What you give it | What it sends |
|---|---|---|
| **E-mail** | addresses, separated by commas | a plain-text e-mail, through the hub's mail server (below) |
| **Webhook** | a URL, and a signing key | a signed JSON POST (below), for your own tools or an automation platform |
| **Microsoft Teams** | a Teams Workflows webhook URL (*Post to a channel when a webhook request is received*) | an Adaptive Card |
| **Slack** | an incoming webhook URL | a message |
| **Discord** | a channel webhook URL (*Edit channel › Integrations › Webhooks*) | an embed, red, amber or green |
| **Telegram** | a bot token (from @BotFather) and a chat ID | a message from the bot |
| **ntfy** | a topic URL on ntfy.sh or your own server, and an access token if it needs one | a push notification on the phones subscribed to the topic |

**Secrets are write-only.** A chat webhook URL, a webhook's signing key, a bot token and the mail server's password are kept encrypted in the database with the hub's data key (`hub_data.key`, [security.md](../security.md)); the page shows only that one is set. To change one, type the new one; leave the field empty to keep it.

**Send test** sends a made-up alert, marked TEST, to that channel at once, and says whether it got through and why not. Each channel shows its **last delivery** and, if it failed, the error the receiver gave (an error never shows a URL: for chat channels, the URL is the secret).

A channel can be **disabled** (it keeps its rules, and gets nothing) and removed; removing it removes it from the rules, and a rule left with no channel goes.

**Delivery.** Alerts are sent in the background; one that fails is tried again after 1 minute, then after 4 more (3 tries), and the channel records the last error. Retries are kept in memory: a hub restart drops them.

### Telegram

1. In Telegram, talk to **@BotFather**: `/newbot`, and keep the token it gives you (`123456789:AA…`).
2. Add the bot to the group (or channel) that should get alerts, and send it a message there.
3. Open `https://api.telegram.org/bot<token>/getUpdates` in a browser; the chat's `"id"` is in the answer (groups are negative, e.g. `-1001234567890`).
4. Add a Telegram channel with the token and that chat ID, and press **Send test**.

### ntfy

[ntfy](https://ntfy.sh) puts alerts on phones (D-70). The channel's **topic URL** is the server and the topic: `https://ntfy.sh/blentinel-1f0c…` on the public server, or `https://ntfy.example.com/blentinel-alerts` on your own. **New random topic** fills in a topic nobody can guess (on the server already typed, or ntfy.sh): on a server anyone can read, the topic name is the only lock, and the form warns about a short one. The **access token** is for a server that needs an account (empty: none). Outages arrive as urgent, warnings and probes that stopped reporting as high, recoveries as normal; reminders and escalations as urgent; with the hub's address set, tapping one opens the incident.

Which server, and how to run your own: [install/ntfy.md](../install/ntfy.md).

### Webhook signatures

A webhook gets a JSON body:

```json
{ "event": "down", "subject": "[BLENTINEL #12][acme][hq] NAS DOWN", "at": "2026-10-07T09:30:00Z",
  "company": { "id": "acme", "name": "ACME Ltd" }, "site": { "id": "hq", "name": "Head office" },
  "probe": { "id": "…", "name": "hq-probe", "hostname": "hq-pc" },
  "check": { "id": "…", "name": "NAS", "kind": "ping", "target": "10.0.0.20" },
  "detail": "no reply",
  "incident": { "id": 12, "ref": "BLENTINEL #12", "state": "open", "opened_at": "2026-10-07T09:30:00Z" },
  "link": "https://hub.example.com/incidents/12" }
```

`event` is `down`, `warning`, `recovered`, `probe_expired`, `reminder` or `escalation` ([D-69](#reminders-and-escalation); `kind` is then the alert's: `down`, `warning`, `probe_expired`), or `test` for **Send test**; `check` is `null` for a probe's alert, `link` without the hub's address. `incident` is the alert's [incident](#incidents) (`null` for a test): `id`, `ref` (`BLENTINEL #12`, as in the subject), `state` (`open`, or `closed` for its recovery) and `opened_at` (a recovery's outage lasted from `opened_at` to `at`). Two headers prove it came from your hub:

- `X-Blentinel-Timestamp`: the Unix time it was sent;
- `X-Blentinel-Signature`: `sha256=` and the hex HMAC-SHA256 of `<timestamp>.<body>` with the signing key.

The receiver computes the same HMAC over the raw body as received, compares it in constant time, and refuses a timestamp more than 5 minutes old (so a captured request can't be replayed). In Python:

```python
import hashlib, hmac, time

def verify(key: bytes, timestamp: str, body: bytes, signature: str) -> bool:
    expected = "sha256=" + hmac.new(key, timestamp.encode() + b"." + body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, signature) and abs(time.time() - int(timestamp)) < 300
```

## Routing rules

Under **Routing rules**, each rule says which alerts it matches, and the channels they go to:

- **Alerts:** Down, Warning, Recovered, Probe stopped reporting;
- **Companies** and **Sites**;
- **Check kinds** (`ping`, `disk`, …; a rule with check kinds doesn't match a probe's alert);
- **Send to:** one or more channels;
- **Stop here when it matches:** later rules aren't looked at.

Nothing ticked in a list means *all*. An alert goes to the channels of **every** rule it matches, from the top, until one that matches says stop; a channel named twice gets it once. No rule matches: it goes nowhere. Use **↑** and **↓** to order them, and **Save rules**.

For example: *1. Alerts: Down; Companies: ACME; Send to: on-call (Telegram); stop.* *2. Send to: ops (e-mail).* ACME's outages page the on-call phone; everything else, ACME's warnings and recoveries included, goes to the ops mailbox.

### Reminders and escalation

An alert at 02:00 that nobody sees shouldn't wait for the morning (DECISIONS D-69). Each rule can say, **while nobody acknowledges** the [incident](#incidents):

- **remind every _N_ minutes** (15 to 10080, a week): the rule's channels get it again, `[BLENTINEL #12][acme][hq] NAS STILL DOWN`, with how long it has been open and which reminder it is;
- **escalate after _M_ minutes** (5 to 1440, a day) **to** other channels, once: `… NAS DOWN (escalated: not acknowledged for 1 h)`. When it recovers, the escalation's channels hear that too, so whoever was woken knows it's over.

Empty: never. **Acknowledging stops both**, and so does recovery. Nothing is sent while a silence or a maintenance window covers it (it resumes when that ends), and never about an outage that started silenced: nobody was told the first time. When an alert matches several rules, it is reminded at the shortest interval, to every reminding rule's channels, and escalated once, at the earliest time, to every escalating rule's channels. Each one is on the incident's timeline (*Reminder 2 to on-call*, *Not acknowledged for 1 h: escalated to manager*); webhooks get `event` `reminder` or `escalation`, with `kind` the alert's.

The hub works it out from the incidents every few seconds, so a restart neither loses nor repeats one. For example: *Alerts: Down; Send to: on-call; remind every 30 minutes; escalate after 60 minutes to: manager.*

## Tickets in your PSA

A PSA (HaloPSA, ConnectWise, Autotask, Syncro, …) can turn the hub's alerts into tickets without anything PSA-specific in the hub (DECISIONS D-72):

- **By e-mail:** most PSAs open a ticket from each e-mail to a mailbox. Add an **e-mail channel** to that mailbox and a **routing rule** for the alerts that should become tickets (say, Down and Probe stopped reporting, of the companies you bill). Every alert about one outage has the same tag in its subject, `[BLENTINEL #12]`: the first, its changes, [reminders and escalation](#reminders-and-escalation), and its recovery. Set the PSA's mail rules to add an e-mail whose subject has a tag it has seen to that ticket (and, if you like, to close it on `RECOVERED`).
- **By webhook:** a PSA (or an automation tool in front of it) that takes webhooks gets the [signed webhook](#webhook-signatures) with `incident.ref` and `incident.state`: open a ticket for a new `ref`, update it while `open`, close it on `closed`.

The tag is the incident's number on this hub: two hubs mailing one PSA can both have a `#12`, so give each its own mailbox or rule.

Direct connectors to a PSA's own API, and SMS, are left for later (ROADMAP M3-10, M3-11: deferred).

## Daily digest

Once a day, one e-mail with how things stand (DECISIONS D-71): on the **Alerts** page, under **Daily digest**, choose the hour, the time zone (`Europe/Madrid`, `UTC`) and the **e-mail channels** it goes to. No channel ticked: no digest. It needs the mail server (below).

It says:

- **open incidents**, each with how long it has been open and who acknowledged it (or **NOT ACKNOWLEDGED**, or *silenced*);
- **probes not reporting**, with their last report;
- **the last 24 hours:** how many incidents opened and closed, and the closed ones, longest first (at most 20);
- **uptime over 24 hours** for each company (archived ones left out);
- with the hub's address set, a link to its incidents.

The subject sums it up: `[BLENTINEL] Daily digest 2026-10-08: 2 open incidents (1 not acknowledged), 1 probe not reporting`, or `… all well`. It goes **every day, also when all is well**: if it stops arriving, look at the hub itself (or its mail server). It goes once per day at that local hour; a hub that was down then sends it when it is back the same day, and a restart doesn't send it twice. **Send digest now** sends one at once, to try it (it doesn't count as the day's). Changes are in the audit log (`alerts.digest`).

## Settings

- **Hub address** (e.g. `https://hub.example.com`): alerts link to the probe's page there. Empty: no links.
- **Mail server**, for e-mail channels: server, port, security (**STARTTLS**, usually port 587; **TLS**, usually 465; **None**, for a relay on the same network, port 25), username and password (if it needs a login), and the From address.

Changes to channels, rules and settings are in the audit log (`channel.create`, `channel.update`, `channel.delete`, `alerts.routes`, `alerts.settings`); secrets never are.

## Incidents

Each outage is an **incident** (DECISIONS D-67): from the alert that starts it to the one that ends it. **Incidents** in the menu lists the open ones (and, under **Closed** and **All**, the earlier ones), with how long each has lasted and who acknowledged it.

- **A check's incident** opens with its DOWN or WARNING alert and closes with RECOVERED. Going from Warning to Down (or back) is the same incident: the list shows how it is now, and its page also the worst it got. The next outage is a new incident.
- **A probe that stopped reporting** is one incident (not one per check), closed when the probe reports again.
- **What counts is the news, not who was told.** A silenced check still opens an incident (its timeline says nobody was told); flap suppression and maintenance windows apply as for alerts, so a blip or work in a window opens none.
- **Deleting a check** closes its incident.

An incident's page shows where it is (with links to the company and the probe), when it opened and closed, and its **timeline**: opened, changed, acknowledged, notes, closed, each with when and who.

**Acknowledge** ("I'm on it") says who is dealing with it, with an optional note; once per incident, while it is open. **Add note** adds to the timeline, open or closed (what was found, what was done). Technicians and up do both (as for silences); Viewers see them. **Clients** see their companies' incidents and timelines, but not the notes or what an acknowledgement said. Acknowledging stops the [reminders and escalation](#reminders-and-escalation). Both are in the audit log (`incident.acknowledge`, `incident.note`).

Deleting a probe's or a company's data (admin page) deletes their incidents too.

## Statuses

The hub gives each check result one of four statuses (DECISIONS D-16, D-64), the same on the dashboard, the probe page, in uptime and in alerts:

| Status | When |
|---|---|
| **Up** | it worked, and is under its thresholds |
| **Warning** | it works, but has reached a **warning** limit: look at it. Counts as up in uptime |
| **Down** | it failed (the probe says so), or has reached a **down** limit |
| **Unknown** | its probe stopped reporting: the last result is history. Its probe alerts once (PROBE EXPIRED), not each check. Or a check it is [behind](#dependencies) is down: it is unreachable |

A company or probe with a check in Warning shows amber on the dashboard; one whose probe stopped reporting shows grey (degraded).

## Thresholds

Limits apply to the **usage** of `cpu`, `memory` and `disk` checks (percent) and the **response time** of `ping`, `http` and `tcp` checks (milliseconds). Each has a warning limit and a down limit; at or over one, the check is Warning or Down.

A `ping` check is judged by two things: its latency, and the **share of its echoes that were lost**. The worse of the two counts. The loss limits are set for the hub, a company or a site (a site on a radio link can allow more), not on a check: a check's own limits are its latency's. No answer at all is Down whatever the limits say.

A latency limit is how you say "this page must answer within 2 seconds": an `http` check has no setting of its own for it.

A **file** check (`file`) has limits on the **age of its newest file**, in hours: a backup that is late, then one that was missed ([checks.md](../reference/checks.md#files-and-folders)).

A **TLS certificate** check (`tls`) has limits too, on the **days its certificate has left**, and they work the other way round: at or **under** one, the check is Warning or Down, so the warning limit is the higher number. An expired certificate is Down whatever the limits say ([checks.md](../reference/checks.md#tls-certificates)).

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
| Packet loss | 50 % | off |
| Certificate | 21 days left | 7 days left |
| File age | 26 hours | 50 hours |
| Temperature | 90 °C | off |

The certificate defaults suit certificates renewed by hand, and automatic ones too: Let's Encrypt's are renewed with about 30 days left, so 21 means "the renewal is a week late". For a certificate that lives only a few days, set the check's own limits, or `off`.

So a company can be stricter about disks, one site with a slow link can allow 500 ms where the rest warn at 200 ms, and one check can switch a limit **off** (`off`) that the levels above set. An empty limit is inherited: the form shows what applies then as a grey placeholder, and where it comes from when you point at it. Changes apply from each probe's next report (results already stored keep the status they had). Technicians set a company's and its sites'; Owners and Admins the hub's. Changes are in the audit log (`thresholds.update`).

## Several results in a row (flap suppression)

A check alerts only when **several results in a row** agree (DECISIONS D-63): by default 2 to go down, 2 to recover. One lost ping, one slow page or one CPU spike alerts nobody, and a link that comes and goes doesn't send DOWN and RECOVERED every other minute. A real outage is reported one probe interval later than the first failure (with the default 60 s interval: about 2 minutes after it starts).

Each check can say otherwise: on the probe's page, **Edit** the check and fill **Alert after _N_ failed results in a row; recovered after _M_ good ones** (1 to 10; empty: the default, 2). 1 alerts on the very first failure, as before. A higher number suits checks that fail now and then on their own (a ping over a busy Wi-Fi link); count it in reports: with a 5-minute interval, 3 failures is a quarter of an hour. Warning counts like Down for going in (failed results), like a recovery for going back to Up. They are hub settings only: changing them doesn't send the probe a new config.

The results are counted in the order the probe made them, also when it delivers a batch it buffered while the hub was unreachable.

Alert state belongs to the check (its ID, D-49): renaming a check or changing its target keeps it, and a new check starts fresh.

## Dependencies

When a site's router or internet link dies, everything behind it fails too: one cause, a dozen alerts. Say what each check is **behind** and you get one (DECISIONS D-68). On the probe's page, **Edit** a check and choose **Behind**: another check of the same probe, usually the gateway's ping (a switch can be behind the router, a NAS behind the switch).

While the check it is behind is **Down** (in that same report):

- its results are **Unknown**, with the reason `unreachable: Gateway is down` on the probe's page;
- it sends **no alerts** and opens **no incident**, and its results **don't count for uptime** (they say nothing about it);
- the DOWN alert and the incident of the check it is behind **name it**: `no reply · unreachable behind it: NAS, Printer`. With a chain, the farthest check that is down is the cause.

It goes by that report's result, not by alerts: one lost gateway ping doesn't count as a failure of the checks behind it either. When the gateway is back, each check is judged on its own again; one still failing alerts after its results in a row, as usual.

A check can't be behind itself, or behind a check that is (directly or not) behind it, or behind another probe's check. Deleting the check others are behind leaves them behind nothing. Copying a probe's checks keeps the links between the copies. It's a hub setting: the probe isn't sent a new config.

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

1. On the **Alerts** page, add a channel you can read and press **Send test**: you get a made-up alert marked TEST (subject `[BLENTINEL][example][hq] Test check TEST`). If it fails, the page says why.
2. Add a rule that sends to it.
3. Give a probe a check that fails, e.g. a `tcp` check on a closed port ([checks.md](../reference/checks.md)). Within two probe intervals you get a DOWN alert (set the check's **Alert after** to 1 to get it after one).

## Coming later

direct PSA connectors and SMS (deferred: M3-10, M3-11), digests for one company (to send to a customer); acknowledging from the alert itself, on-call schedules and quiet hours: see [proposal 0005](../proposals/0005-alerting-v2.md) and the [roadmap](../ROADMAP.md). A new channel is one more `ChannelKind` and its shape in `hub/src/service/notify/payload.rs`.
