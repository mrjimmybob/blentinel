//! The daily digest (M3-09, proposal 0005 §2.6, D-71): once a day, at an
//! hour local to a time zone, an e-mail to the chosen e-mail channels with
//! what is open, what happened in the last 24 hours, which probes are silent
//! and each company's uptime. Also on a quiet day: a digest that stops
//! arriving says the hub itself is in trouble.
//!
//! [`sweep`] runs with the other background checks and keeps the local date
//! it last went out in the database: a restart neither repeats nor skips it,
//! and a hub that was down at that hour sends it once it is back that day.

use anyhow::Result;
use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;

use crate::model::{ChannelKind, CompanyScope, DigestSettings, Status, TestResult, duration_text};
use crate::service::maintenance::zone;
use crate::service::notify::Delivery;
use crate::storage::{Storage, StoredIncident, UptimeRange};

/// The longest list of closed incidents a digest shows.
const MAX_CLOSED: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum DigestError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// A digest, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub subject: String,
    pub body: String,
}

/// A probe that stopped reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Silent {
    pub company: String,
    pub probe: String,
    pub last_seen: Option<DateTime<Utc>>,
}

/// What a digest says, gathered.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Open incidents, oldest first.
    pub open: Vec<StoredIncident>,
    /// Incidents opened in the last 24 hours.
    pub opened: usize,
    /// Incidents closed in the last 24 hours.
    pub closed: Vec<StoredIncident>,
    pub silent: Vec<Silent>,
    /// Each company and its uptime over 24 hours (`None`: no results).
    pub uptime: Vec<(String, Option<f64>)>,
}

/// Gather the facts for the 24 hours up to `now`.
pub async fn gather(store: &dyn Storage, now: DateTime<Utc>) -> Result<Facts> {
    let since = now - Duration::hours(24);
    let mut open = store
        .incidents(&CompanyScope::All, Some(true), 1000)
        .await?;
    open.reverse();
    let recent = store.incidents(&CompanyScope::All, None, 1000).await?;
    let opened = recent.iter().filter(|i| i.opened_at >= since).count();
    let mut closed: Vec<StoredIncident> = recent
        .into_iter()
        .filter(|i| i.closed_at.is_some_and(|c| c >= since))
        .collect();
    closed.sort_by_key(|i| std::cmp::Reverse(i.closed_at.unwrap_or(now) - i.opened_at));

    // The company records (not the dashboard's list, which leaves out a
    // company whose probes all went quiet); archived ones aren't watched.
    let companies: Vec<_> = store
        .companies(&CompanyScope::All)
        .await?
        .into_iter()
        .filter(|c| !c.archived)
        .collect();
    let name_of = |id: &str| {
        companies
            .iter()
            .find(|c| c.id == id)
            .map_or_else(|| id.to_string(), |c| c.name.clone())
    };
    let mut silent = Vec::new();
    for probe in store.all_probes().await? {
        if probe.status != "expired" {
            continue;
        }
        let name = match store.probe_record(&probe.probe_id).await? {
            Some(r) if !r.name.is_empty() => r.name,
            _ => crate::model::short_probe_id(&probe.probe_id).to_string(),
        };
        silent.push(Silent {
            company: name_of(&probe.company_id),
            probe: name,
            last_seen: probe
                .last_seen_at
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc)),
        });
    }
    let mut uptime = Vec::new();
    for company in &companies {
        let buckets = store
            .company_uptime(&company.id, UptimeRange::Hours24, now)
            .await?;
        let (up, down) = buckets
            .iter()
            .fold((0, 0), |(u, d), b| (u + b.up_count, d + b.down_count));
        let percent = (up + down > 0).then(|| up as f64 * 100.0 / (up + down) as f64);
        uptime.push((company.name.clone(), percent));
    }
    Ok(Facts {
        open,
        opened,
        closed,
        silent,
        uptime,
    })
}

fn local(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string()
}

/// `ACME Ltd › Head office › hq-probe › NAS`.
fn place(i: &StoredIncident) -> String {
    let mut parts = vec![
        i.company_name.as_str(),
        i.site_name.as_str(),
        i.probe_name.as_str(),
    ];
    match &i.check_name {
        Some(check) => parts.push(check),
        None => parts.push("(stopped reporting)"),
    }
    parts.join(" › ")
}

/// `99.2 %` (no more digits than say something).
fn percent(p: f64) -> String {
    if p >= 100.0 {
        "100 %".into()
    } else {
        format!("{:.1} %", (p * 10.0).floor() / 10.0)
    }
}

/// The digest of `facts` at `now` in `tz`, linking to `hub_url` if set.
pub fn render(facts: &Facts, now: DateTime<Utc>, tz: Tz, hub_url: &str) -> Digest {
    let date = now.with_timezone(&tz).format("%Y-%m-%d");
    let unacknowledged = facts
        .open
        .iter()
        .filter(|i| i.acknowledged_by.is_none())
        .count();
    let mut news = Vec::new();
    if !facts.open.is_empty() {
        let mut open = format!(
            "{} open incident{}",
            facts.open.len(),
            if facts.open.len() == 1 { "" } else { "s" }
        );
        if unacknowledged > 0 {
            open.push_str(&format!(" ({unacknowledged} not acknowledged)"));
        }
        news.push(open);
    }
    if !facts.silent.is_empty() {
        news.push(format!(
            "{} probe{} not reporting",
            facts.silent.len(),
            if facts.silent.len() == 1 { "" } else { "s" }
        ));
    }
    let headline = match news.is_empty() {
        true => "all well".to_string(),
        false => news.join(", "),
    };
    let subject = format!("[BLENTINEL] Daily digest {date}: {headline}");

    let mut b = vec![
        format!("Blentinel daily digest, {} {}", local(now, tz), tz.name()),
        format!(
            "The 24 hours since {}: {} incident{} opened, {} closed.",
            local(now - Duration::hours(24), tz),
            facts.opened,
            if facts.opened == 1 { "" } else { "s" },
            facts.closed.len()
        ),
        String::new(),
    ];
    b.push(format!("OPEN INCIDENTS ({})", facts.open.len()));
    if facts.open.is_empty() {
        b.push("None.".into());
    }
    for i in &facts.open {
        let who = match (&i.acknowledged_by, i.announced) {
            (Some(by), _) => format!("acknowledged by {by}"),
            (None, false) => "silenced".into(),
            (None, true) => "NOT ACKNOWLEDGED".into(),
        };
        b.push(format!(
            "- {} {}: for {}, {who}",
            i.status.as_str().to_uppercase(),
            place(i),
            duration_text((now - i.opened_at).num_seconds())
        ));
    }
    b.push(String::new());
    b.push(format!("PROBES NOT REPORTING ({})", facts.silent.len()));
    if facts.silent.is_empty() {
        b.push("None.".into());
    }
    for s in &facts.silent {
        let since = s.last_seen.map_or_else(
            || "never reported".to_string(),
            |t| format!("last report {}", local(t, tz)),
        );
        b.push(format!("- {} › {}: {since}", s.company, s.probe));
    }
    b.push(String::new());
    b.push(format!(
        "CLOSED IN THE LAST 24 HOURS ({})",
        facts.closed.len()
    ));
    if facts.closed.is_empty() {
        b.push("None.".into());
    }
    for i in facts.closed.iter().take(MAX_CLOSED) {
        let end = i.closed_at.unwrap_or(now);
        let worst = match i.severity {
            Status::Warning => "warning",
            _ => "down",
        };
        b.push(format!(
            "- {}: {worst} for {}, until {}",
            place(i),
            duration_text((end - i.opened_at).num_seconds()),
            local(end, tz)
        ));
    }
    if facts.closed.len() > MAX_CLOSED {
        b.push(format!("  … and {} more", facts.closed.len() - MAX_CLOSED));
    }
    b.push(String::new());
    b.push("UPTIME, LAST 24 HOURS".into());
    if facts.uptime.is_empty() {
        b.push("No companies yet.".into());
    }
    for (company, p) in &facts.uptime {
        let p = p.map_or_else(|| "no results".to_string(), percent);
        b.push(format!("- {company}: {p}"));
    }
    let base = hub_url.trim().trim_end_matches('/');
    if !base.is_empty() {
        b.push(String::new());
        b.push(format!("{base}/incidents"));
    }
    Digest {
        subject,
        body: b.join("\n"),
    }
}

/// The digest's settings and the e-mail channels it may use.
async fn email_channels(store: &dyn Storage) -> Result<Vec<i64>> {
    Ok(store
        .channels()
        .await?
        .into_iter()
        .filter(|c| c.kind == ChannelKind::Email)
        .map(|c| c.id)
        .collect())
}

/// Save the digest's settings.
pub async fn save(
    store: &dyn Storage,
    settings: &DigestSettings,
    by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), DigestError> {
    settings
        .validate(&email_channels(store).await?)
        .map_err(DigestError::Invalid)?;
    zone(&settings.timezone).map_err(DigestError::Invalid)?;
    store.set_digest_settings(settings, by, now).await?;
    Ok(())
}

/// Compose the digest at `now` and send it to `settings`' channels (each
/// records how it went); `Err` lists the ones that failed.
async fn send(
    store: &dyn Storage,
    delivery: &Delivery,
    settings: &DigestSettings,
    now: DateTime<Utc>,
) -> Result<TestResult> {
    let tz = zone(&settings.timezone).map_err(|e| anyhow::anyhow!(e))?;
    let hub_url = store
        .alert_settings()
        .await?
        .map(|s| s.hub_url)
        .unwrap_or_default();
    let digest = render(&gather(store, now).await?, now, tz, &hub_url);
    let mut failed = Vec::new();
    for id in &settings.channels {
        let Some(channel) = store.channel(*id).await? else {
            continue;
        };
        if !channel.enabled {
            continue;
        }
        if let Err(e) = delivery
            .send_mail(store, &channel, &digest.subject, &digest.body)
            .await
        {
            failed.push(format!("{}: {e:#}", channel.name));
        }
    }
    Ok(match failed.is_empty() {
        true => TestResult {
            ok: true,
            error: None,
        },
        false => TestResult {
            ok: false,
            error: Some(failed.join("; ")),
        },
    })
}

/// "Send digest now": the digest as it would go, now (not counted as the
/// day's).
pub async fn send_now(
    store: &dyn Storage,
    delivery: &Delivery,
    now: DateTime<Utc>,
) -> Result<TestResult, DigestError> {
    let settings = store
        .digest_settings()
        .await?
        .map(|(s, _)| s)
        .unwrap_or_default();
    if settings.channels.is_empty() {
        return Err(DigestError::Invalid(
            "choose the e-mail channels it goes to, and save".into(),
        ));
    }
    Ok(send(store, delivery, &settings, now).await?)
}

/// Send the day's digest if it is due at `now`: past its hour (local), and
/// not sent yet that local day. Recorded before it is sent, so a slow mail
/// server can't make it twice.
pub async fn sweep(store: &dyn Storage, delivery: &Delivery, now: DateTime<Utc>) -> Result<()> {
    let Some((settings, last_sent)) = store.digest_settings().await? else {
        return Ok(());
    };
    if settings.channels.is_empty() {
        return Ok(());
    }
    let tz = zone(&settings.timezone).map_err(|e| anyhow::anyhow!(e))?;
    let here = now.with_timezone(&tz);
    let today = here.format("%Y-%m-%d").to_string();
    if here.hour() < u32::from(settings.hour) || last_sent.as_deref() == Some(today.as_str()) {
        return Ok(());
    }
    if !store.record_digest_sent(&today).await? {
        return Ok(());
    }
    let result = send(store, delivery, &settings, now).await?;
    if let Some(error) = result.error {
        tracing::error!("The daily digest wasn't delivered everywhere: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn incident(check: Option<&str>, opened: &str, closed: Option<&str>) -> StoredIncident {
        StoredIncident {
            id: 1,
            company_id: "acme".into(),
            company_name: "ACME Ltd".into(),
            site_id: "hq".into(),
            site_name: "Head office".into(),
            probe_id: "p".into(),
            probe_name: "hq-probe".into(),
            check_id: check.map(str::to_string),
            check_name: check.map(str::to_string),
            severity: Status::Down,
            status: Status::Down,
            opened_at: at(opened),
            closed_at: closed.map(at),
            acknowledged_by: None,
            acknowledged_at: None,
            announced: true,
            reminders: 0,
            last_reminded_at: None,
            escalated_at: None,
        }
    }

    #[test]
    fn a_quiet_day_says_all_well() {
        let facts = Facts {
            uptime: vec![("ACME Ltd".into(), Some(100.0)), ("Zeta".into(), None)],
            ..Facts::default()
        };
        let digest = render(
            &facts,
            at("2026-10-08T05:00:00Z"),
            chrono_tz::Europe::Madrid,
            "",
        );
        assert_eq!(
            digest.subject,
            "[BLENTINEL] Daily digest 2026-10-08: all well"
        );
        assert!(
            digest
                .body
                .starts_with("Blentinel daily digest, 2026-10-08 07:00 Europe/Madrid\n")
        );
        assert!(digest.body.contains("OPEN INCIDENTS (0)\nNone."));
        assert!(
            digest
                .body
                .contains("- ACME Ltd: 100 %\n- Zeta: no results")
        );
        assert!(
            !digest.body.contains("http"),
            "no link without the hub's address"
        );
    }

    #[test]
    fn a_busy_day_lists_what_matters() {
        let now = at("2026-10-08T07:00:00Z");
        let mut acked = incident(Some("NAS"), "2026-10-08T04:00:00Z", None);
        acked.acknowledged_by = Some("alice".into());
        let facts = Facts {
            open: vec![acked, incident(None, "2026-10-08T06:30:00Z", None)],
            opened: 3,
            closed: vec![incident(
                Some("Printer"),
                "2026-10-07T20:00:00Z",
                Some("2026-10-07T20:45:00Z"),
            )],
            silent: vec![Silent {
                company: "ACME Ltd".into(),
                probe: "hq-probe".into(),
                last_seen: Some(at("2026-10-08T06:25:00Z")),
            }],
            uptime: vec![("ACME Ltd".into(), Some(99.86))],
        };
        let digest = render(&facts, now, chrono_tz::UTC, "https://hub.example.com/");
        assert_eq!(
            digest.subject,
            "[BLENTINEL] Daily digest 2026-10-08: 2 open incidents (1 not acknowledged), 1 probe not reporting"
        );
        let body = &digest.body;
        assert!(body.contains("3 incidents opened, 1 closed."), "{body}");
        assert!(body.contains(
            "- DOWN ACME Ltd › Head office › hq-probe › NAS: for 3 h, acknowledged by alice"
        ));
        assert!(body.contains("› (stopped reporting): for 30 min, NOT ACKNOWLEDGED"));
        assert!(body.contains("- ACME Ltd › hq-probe: last report 2026-10-08 06:25"));
        assert!(body.contains("› Printer: down for 45 min, until 2026-10-07 20:45"));
        assert!(
            body.contains("- ACME Ltd: 99.8 %"),
            "rounded down: never better than it was"
        );
        assert!(body.ends_with("\nhttps://hub.example.com/incidents"));
    }
}
