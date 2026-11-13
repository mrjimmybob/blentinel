//! The monthly availability report of a company (M6-05, proposal 0007 §3.4,
//! D-113): per site and per check, availability and the month before,
//! incidents with their duration and cause, maintenance, as HTML (the page)
//! and CSV.
//!
//! Each site's figures cover its own local month, summed from the hourly
//! rollups (D-111), which line up with any whole-hour time zone. A month
//! older than the hourly rollups are kept is read from the daily (UTC) ones,
//! and the report says it is approximate.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

use crate::model::{
    CheckReport, CompanyScope, IncidentEventKind, MaintenanceWindow, MonthlyReport, ReportIncident,
    ReportMaintenance, ReportMonth, Scope, SiteReport, StatusCounts,
};
use crate::service::{history, maintenance, scope};
use crate::storage::{CheckTotals, RollupWidth, Storage, StoredIncident};

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// Unknown, or outside the user's companies (deliberately the same).
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReportRequest {
    pub month: ReportMonth,
    pub target_pct: Option<f64>,
}

/// The month in UTC that `now` is in.
pub fn current_month(now: DateTime<Utc>) -> ReportMonth {
    use chrono::Datelike;
    ReportMonth {
        year: now.year(),
        month: now.month(),
    }
}

impl ReportRequest {
    /// From `?month=YYYY-MM` (default: the month before this one) and
    /// `?target=` (a percentage).
    pub fn parse(
        month: Option<&str>,
        target: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Self, ReportError> {
        let invalid = |text: &str| ReportError::Invalid(text.into());
        let this_month = current_month(now);
        let month = match month.map(str::trim).filter(|m| !m.is_empty()) {
            None => this_month.previous(),
            Some(text) => ReportMonth::parse(text)
                .ok_or_else(|| invalid("the month is YYYY-MM, e.g. 2026-10"))?,
        };
        if month > this_month {
            return Err(invalid("that month hasn't started"));
        }
        let target_pct = match target.map(str::trim).filter(|t| !t.is_empty()) {
            None => None,
            Some(text) => match text.trim_end_matches('%').parse::<f64>() {
                Ok(pct) if pct > 0.0 && pct <= 100.0 => Some(pct),
                _ => return Err(invalid("the target is a percentage above 0, at most 100")),
            },
        };
        Ok(Self { month, target_pct })
    }
}

/// When `month` starts in `tz` (its first local midnight), in UTC.
pub fn month_start(month: ReportMonth, tz: Tz) -> DateTime<Utc> {
    let midnight = NaiveDate::from_ymd_opt(month.year, month.month, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default();
    maintenance::to_utc(tz, midnight)
}

/// `month` in `tz`, `[from, to)`, ending at `now` at the latest.
pub fn month_span(
    month: ReportMonth,
    tz: Tz,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let from = month_start(month, tz);
    let to = month_start(month.next(), tz).min(now);
    (from, to.max(from))
}

/// How long `[start, end)` overlaps `[from, to)`, in seconds.
pub fn overlap_secs(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> i64 {
    (end.min(to) - start.max(from)).num_seconds().max(0)
}

/// How many of `incidents` opened in `[from, to)`, and how long they were
/// open in it, added up (an open one is open until `now`).
pub fn incident_figures<'a>(
    incidents: impl IntoIterator<Item = &'a StoredIncident>,
    (from, to): (DateTime<Utc>, DateTime<Utc>),
    now: DateTime<Utc>,
) -> (u32, i64) {
    let mut opened = 0u32;
    let mut secs = 0i64;
    for i in incidents {
        if from <= i.opened_at && i.opened_at < to {
            opened += 1;
        }
        secs += overlap_secs(i.opened_at, i.closed_at.unwrap_or(now), from, to);
    }
    (opened, secs)
}

fn minute_in(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string()
}

fn minute(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

/// A site's zone (UTC if it isn't one).
fn zone(name: &str) -> Tz {
    maintenance::zone(name).unwrap_or(Tz::UTC)
}

/// Where a check belongs: its probe and site, its name and kind.
struct CheckPlace {
    probe_name: String,
    site_id: String,
    name: String,
    kind: String,
}

/// A company of the user's, for `request`.
pub async fn monthly_report(
    store: &dyn Storage,
    user_scope: &CompanyScope,
    company_id: &str,
    request: ReportRequest,
    now: DateTime<Utc>,
) -> Result<MonthlyReport, ReportError> {
    scope::require(user_scope, company_id).map_err(|_| ReportError::NotFound)?;
    let company = store
        .company(company_id)
        .await?
        .ok_or(ReportError::NotFound)?;
    let month = request.month;

    // The company's probes and checks as they are now.
    let only = CompanyScope::Only(BTreeSet::from([company_id.to_string()]));
    let probes = store.probe_records(&only).await?;
    let mut places: HashMap<String, CheckPlace> = HashMap::new();
    let mut probe_site: HashMap<String, (String, String)> = HashMap::new();
    for probe in &probes {
        probe_site.insert(
            probe.id.clone(),
            (probe.name.clone(), probe.site_id.clone()),
        );
        for check in store.probe_checks(&probe.id).await? {
            places.insert(
                check.id.0.clone(),
                CheckPlace {
                    probe_name: probe.name.clone(),
                    site_id: probe.site_id.clone(),
                    name: check.name.clone(),
                    kind: check.spec.kind().to_string(),
                },
            );
        }
    }

    // The finest rollups kept for each month: hourly, else daily (UTC).
    let retention = store.history_retention().await?;
    let hourly_since = history::cutoffs(&retention, now).hourly;
    let width_from = |from: DateTime<Utc>| {
        if from >= hourly_since {
            RollupWidth::Hourly
        } else {
            RollupWidth::Daily
        }
    };

    // Each site's month and the one before, in its zone; read once per zone.
    let mut by_zone: HashMap<String, (Vec<CheckTotals>, Vec<CheckTotals>)> = HashMap::new();
    let mut zones: BTreeSet<String> = company.sites.iter().map(|s| s.timezone.clone()).collect();
    zones.insert("UTC".into()); // probes no longer enrolled
    let mut exact = true;
    for name in &zones {
        let tz = zone(name);
        let span = month_span(month, tz, now);
        let before = month_span(month.previous(), tz, now);
        exact &= width_from(span.0) == RollupWidth::Hourly;
        let current = store
            .company_check_totals(company_id, width_from(span.0), span.0, span.1)
            .await?;
        let previous = store
            .company_check_totals(company_id, width_from(before.0), before.0, before.1)
            .await?;
        by_zone.insert(name.clone(), (current, previous));
    }

    // Incidents of the widest span (every zone's month).
    let spans: Vec<_> = zones
        .iter()
        .map(|z| month_span(month, zone(z), now))
        .collect();
    let widest = (
        spans.iter().map(|s| s.0).min().unwrap_or(now),
        spans.iter().map(|s| s.1).max().unwrap_or(now),
    );
    let incidents = store
        .company_incidents(company_id, widest.0, widest.1)
        .await?;

    // One row per site, then the checks of probes no longer enrolled.
    let mut site_rows: Vec<(String, String, String)> = company
        .sites
        .iter()
        .map(|s| (s.id.clone(), s.name.clone(), s.timezone.clone()))
        .collect();
    site_rows.sort_by_key(|site| site.1.to_lowercase());
    site_rows.push((
        String::new(),
        "Probes no longer enrolled".into(),
        "UTC".into(),
    ));

    let mut sites = Vec::new();
    let mut company_before = StatusCounts::default();
    let mut report_incidents = Vec::new();
    for (site_id, site_name, timezone) in site_rows {
        let tz = zone(&timezone);
        let span = month_span(month, tz, now);
        let (current, previous) = &by_zone[if by_zone.contains_key(&timezone) {
            &timezone
        } else {
            "UTC"
        }];
        let site_of = |row: &CheckTotals| {
            probe_site
                .get(&row.probe_id)
                .map_or(String::new(), |(_, site)| site.clone())
        };
        let mut rows: BTreeMap<String, (StatusCounts, StatusCounts, String)> = BTreeMap::new();
        for row in current.iter().filter(|r| site_of(r) == site_id) {
            rows.entry(row.check_id.clone())
                .or_insert_with(|| {
                    (
                        StatusCounts::default(),
                        StatusCounts::default(),
                        row.probe_id.clone(),
                    )
                })
                .0
                .add(&row.counts);
        }
        for row in previous.iter().filter(|r| site_of(r) == site_id) {
            rows.entry(row.check_id.clone())
                .or_insert_with(|| {
                    (
                        StatusCounts::default(),
                        StatusCounts::default(),
                        row.probe_id.clone(),
                    )
                })
                .1
                .add(&row.counts);
        }
        // Checks without results this month are listed too.
        for (id, _) in places.iter().filter(|(_, p)| p.site_id == site_id) {
            rows.entry(id.clone()).or_default();
        }

        let site_incidents: Vec<&StoredIncident> = incidents
            .iter()
            .filter(|i| {
                let here = probe_site
                    .get(&i.probe_id)
                    .map_or(String::new(), |(_, site)| site.clone());
                here == site_id
                    && i.opened_at < span.1
                    && i.closed_at.is_none_or(|closed| closed >= span.0)
            })
            .collect();

        let mut checks: Vec<CheckReport> = rows
            .into_iter()
            .map(|(check_id, (totals, before, probe_id))| {
                let place = places.get(&check_id);
                let own: Vec<&StoredIncident> = site_incidents
                    .iter()
                    .copied()
                    .filter(|i| i.check_id.as_deref() == Some(check_id.as_str()))
                    .collect();
                let (incident_count, downtime_secs) =
                    incident_figures(own.iter().copied(), span, now);
                let known_name = own.iter().find_map(|i| i.check_name.clone());
                CheckReport {
                    name: place
                        .map(|p| p.name.clone())
                        .or(known_name)
                        .unwrap_or_else(|| check_id.clone()),
                    probe_name: place.map(|p| p.probe_name.clone()).unwrap_or_else(|| {
                        probe_site.get(&probe_id).map_or_else(
                            || crate::model::short_probe_id(&probe_id).to_string(),
                            |(n, _)| n.clone(),
                        )
                    }),
                    kind: place.map(|p| p.kind.clone()).unwrap_or_default(),
                    deleted: place.is_none(),
                    availability_pct: totals.availability_pct(),
                    previous_pct: before.availability_pct(),
                    totals,
                    incident_count,
                    downtime_secs,
                    check_id,
                }
            })
            .collect();
        checks.sort_by(|a, b| {
            (a.probe_name.to_lowercase(), a.name.to_lowercase())
                .cmp(&(b.probe_name.to_lowercase(), b.name.to_lowercase()))
        });

        let mut totals = StatusCounts::default();
        let mut before = StatusCounts::default();
        for check in &checks {
            totals.add(&check.totals);
        }
        for row in previous.iter().filter(|r| site_of(r) == site_id) {
            before.add(&row.counts);
        }
        let (incident_count, _) = incident_figures(site_incidents.iter().copied(), span, now);
        let downtime_secs = checks.iter().map(|c| c.downtime_secs).sum();

        for i in &site_incidents {
            report_incidents.push((i.opened_at, i.id, (*i).clone(), site_name.clone()));
        }
        // The pseudo-site only when something is in it.
        if site_id.is_empty() && checks.is_empty() && site_incidents.is_empty() {
            continue;
        }
        company_before.add(&before);
        sites.push(SiteReport {
            site_id,
            name: site_name,
            timezone,
            from: minute_in(span.0, tz),
            to: minute_in(span.1, tz),
            availability_pct: totals.availability_pct(),
            previous_pct: before.availability_pct(),
            totals,
            incident_count,
            downtime_secs,
            checks,
        });
    }

    // Incidents, oldest first, each with what the hub saw when it opened.
    report_incidents.sort_by_key(|(at, id, _, _)| (*at, *id));
    let mut listed = Vec::new();
    for (_, _, incident, site_name) in report_incidents {
        let cause = store
            .incident_events(incident.id)
            .await?
            .into_iter()
            .find(|e| e.kind == IncidentEventKind::Opened)
            .map(|e| e.text)
            .unwrap_or_default();
        listed.push(ReportIncident {
            id: incident.id,
            site_name,
            probe_name: incident.probe_name.clone(),
            check_name: incident.check_name.clone(),
            severity: incident.severity,
            opened_at: minute(incident.opened_at),
            closed_at: incident.closed_at.map(minute),
            duration_secs: (incident.closed_at.unwrap_or(now) - incident.opened_at)
                .num_seconds()
                .max(0),
            cause,
        });
    }

    let maintenance = maintenance_in(
        &store.maintenance_windows(company_id).await?,
        widest,
        &company.sites,
        &probe_site,
        &places,
    );

    let mut totals = StatusCounts::default();
    for site in &sites {
        totals.add(&site.totals);
    }
    let incident_count = sites.iter().map(|s| s.incident_count).sum();
    let downtime_secs = sites.iter().map(|s| s.downtime_secs).sum();

    Ok(MonthlyReport {
        company_id: company_id.to_string(),
        company_name: company.company.name.clone(),
        month,
        generated_at: minute(now),
        partial: month == current_month(now),
        exact,
        target_pct: request.target_pct,
        availability_pct: totals.availability_pct(),
        previous_pct: company_before.availability_pct(),
        totals,
        incident_count,
        downtime_secs,
        sites,
        incidents: listed,
        maintenance,
    })
}

/// The openings of the company's windows that start in `span`.
fn maintenance_in(
    windows: &[MaintenanceWindow],
    span: (DateTime<Utc>, DateTime<Utc>),
    sites: &[crate::model::SiteInfo],
    probes: &HashMap<String, (String, String)>,
    checks: &HashMap<String, CheckPlace>,
) -> Vec<ReportMaintenance> {
    let mut found: Vec<(DateTime<Utc>, ReportMaintenance)> = Vec::new();
    for window in windows {
        let tz = zone(&window.timezone);
        let scope = match &window.scope {
            Scope::Company => "The whole company".to_string(),
            Scope::Site { site_id } => format!(
                "Site {}",
                sites
                    .iter()
                    .find(|s| &s.id == site_id)
                    .map_or(site_id.as_str(), |s| s.name.as_str())
            ),
            Scope::Probe { probe_id } => format!(
                "Probe {}",
                probes
                    .get(probe_id)
                    .map_or(probe_id.as_str(), |(name, _)| name.as_str())
            ),
            Scope::Check { check_id } => format!(
                "Check {}",
                checks
                    .get(check_id)
                    .map_or(check_id.as_str(), |c| c.name.as_str())
            ),
        };
        for (start, end) in maintenance::openings(window, span.0, span.1) {
            found.push((
                start,
                ReportMaintenance {
                    reason: window.reason.clone(),
                    scope: scope.clone(),
                    start: minute_in(start, tz),
                    end: minute_in(end, tz),
                    timezone: window.timezone.clone(),
                },
            ));
        }
    }
    found.sort_by_key(|(start, _)| *start);
    found.into_iter().map(|(_, m)| m).collect()
}

fn pct(value: Option<f64>) -> String {
    value.map(|p| format!("{p:.3}")).unwrap_or_default()
}

/// One row per check.
pub fn to_csv(report: &MonthlyReport) -> String {
    let mut out = String::from(
        "site,probe,check,kind,availability_pct,previous_pct,up,warning,down,unknown,in_maintenance,incidents,downtime_minutes,deleted\r\n",
    );
    for site in &report.sites {
        for c in &site.checks {
            let t = &c.totals;
            let fields = [
                crate::service::csv::field(&site.name),
                crate::service::csv::field(&c.probe_name),
                crate::service::csv::field(&c.name),
                crate::service::csv::field(&c.kind),
                pct(c.availability_pct),
                pct(c.previous_pct),
                t.up.to_string(),
                t.warning.to_string(),
                t.down.to_string(),
                t.unknown.to_string(),
                t.maintenance.to_string(),
                c.incident_count.to_string(),
                (c.downtime_secs / 60).to_string(),
                c.deleted.to_string(),
            ];
            out.push_str(&fields.join(","));
            out.push_str("\r\n");
        }
    }
    out
}

/// `blentinel-acme-2026-10.csv`: the company ID, kept to safe characters.
pub fn csv_filename(report: &MonthlyReport) -> String {
    let company: String = report
        .company_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("blentinel-{company}-{}.csv", report.month.param())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Status;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn incident(opened: &str, closed: Option<&str>) -> StoredIncident {
        StoredIncident {
            id: 1,
            company_id: "acme".into(),
            company_name: "Acme".into(),
            site_id: "hq".into(),
            site_name: "HQ".into(),
            probe_id: "p".into(),
            probe_name: "p".into(),
            check_id: Some("c".into()),
            check_name: Some("c".into()),
            severity: Status::Down,
            status: Status::Down,
            opened_at: t(opened),
            closed_at: closed.map(t),
            acknowledged_by: None,
            acknowledged_at: None,
            announced: true,
            reminders: 0,
            last_reminded_at: None,
            escalated_at: None,
        }
    }

    #[test]
    fn a_request_is_last_month_unless_another_has_started() {
        let now = t("2026-11-12T10:00:00Z");
        let last = ReportRequest::parse(None, None, now).unwrap();
        assert_eq!(last.month.param(), "2026-10");
        assert_eq!(last.target_pct, None);
        let this = ReportRequest::parse(Some("2026-11"), Some("99.9%"), now).unwrap();
        assert_eq!(
            (this.month.param().as_str(), this.target_pct),
            ("2026-11", Some(99.9))
        );
        let january = ReportRequest::parse(None, None, t("2027-01-03T00:00:00Z")).unwrap();
        assert_eq!(january.month.param(), "2026-12");
        for (month, target) in [
            (Some("2026-12"), None),
            (Some("2026-13"), None),
            (Some("Oct"), None),
            (None, Some("0")),
            (None, Some("100.1")),
            (None, Some("high")),
        ] {
            assert!(
                ReportRequest::parse(month, target, now).is_err(),
                "{month:?} {target:?}"
            );
        }
    }

    #[test]
    fn a_month_is_the_sites_local_month_and_ends_now_at_the_latest() {
        let october = ReportMonth::parse("2026-10").unwrap();
        let madrid: Tz = "Europe/Madrid".parse().unwrap();
        let later = t("2027-01-01T00:00:00Z");
        // UTC+2 on the 1st, UTC+1 by the 31st (daylight saving ends the 25th).
        assert_eq!(
            month_span(october, madrid, later),
            (t("2026-09-30T22:00:00Z"), t("2026-10-31T23:00:00Z"))
        );
        assert_eq!(
            month_span(october, Tz::UTC, later),
            (t("2026-10-01T00:00:00Z"), t("2026-11-01T00:00:00Z"))
        );
        let now = t("2026-10-12T10:00:00Z");
        assert_eq!(month_span(october, Tz::UTC, now).1, now, "so far");
        let november = ReportMonth::parse("2026-11").unwrap();
        let (from, to) = month_span(november, Tz::UTC, now);
        assert_eq!(from, to, "not started: empty");
    }

    #[test]
    fn incidents_count_where_they_open_and_downtime_is_their_time_in_the_month() {
        let month = (t("2026-10-01T00:00:00Z"), t("2026-11-01T00:00:00Z"));
        let now = t("2026-11-12T00:00:00Z");
        let incidents = [
            // From September: an hour of it in October, not counted as opened.
            incident("2026-09-30T23:00:00Z", Some("2026-10-01T01:00:00Z")),
            // Inside: 30 minutes.
            incident("2026-10-10T10:00:00Z", Some("2026-10-10T10:30:00Z")),
            // Still open: until the month's end, 2 hours.
            incident("2026-10-31T22:00:00Z", None),
            // November: nothing.
            incident("2026-11-02T00:00:00Z", None),
        ];
        let (opened, secs) = incident_figures(&incidents, month, now);
        assert_eq!(opened, 2);
        assert_eq!(secs, 3600 + 1800 + 7200);
    }

    fn report() -> MonthlyReport {
        let check = |name: &str, availability: Option<f64>| CheckReport {
            check_id: name.into(),
            name: name.into(),
            probe_name: "hq-1".into(),
            kind: "ping".into(),
            deleted: false,
            totals: StatusCounts {
                up: 99,
                down: 1,
                ..StatusCounts::default()
            },
            availability_pct: availability,
            previous_pct: None,
            incident_count: 1,
            downtime_secs: 125,
        };
        MonthlyReport {
            company_id: "acme corp/1".into(),
            company_name: "Acme".into(),
            month: ReportMonth::parse("2026-10").unwrap(),
            generated_at: String::new(),
            partial: false,
            exact: true,
            target_pct: None,
            totals: StatusCounts::default(),
            availability_pct: None,
            previous_pct: None,
            incident_count: 0,
            downtime_secs: 0,
            sites: vec![SiteReport {
                site_id: "hq".into(),
                name: "HQ, Madrid".into(),
                timezone: "Europe/Madrid".into(),
                from: String::new(),
                to: String::new(),
                totals: StatusCounts::default(),
                availability_pct: None,
                previous_pct: None,
                incident_count: 0,
                downtime_secs: 0,
                checks: vec![check("Core switch", Some(99.0)), check("=cmd", None)],
            }],
            incidents: vec![],
            maintenance: vec![],
        }
    }

    #[test]
    fn the_csv_has_a_row_per_check_safe_for_spreadsheets() {
        let csv = to_csv(&report());
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(
            lines[0],
            "site,probe,check,kind,availability_pct,previous_pct,up,warning,down,unknown,in_maintenance,incidents,downtime_minutes,deleted"
        );
        assert_eq!(
            lines[1],
            "\"HQ, Madrid\",hq-1,Core switch,ping,99.000,,99,0,1,0,0,1,2,false"
        );
        assert!(
            lines[2].starts_with("\"HQ, Madrid\",hq-1,'=cmd,ping,,,"),
            "{}",
            lines[2]
        );
        assert_eq!(
            lines.len(),
            4,
            "a header, two rows, and the last line break"
        );
        assert_eq!(csv_filename(&report()), "blentinel-acme_corp_1-2026-10.csv");
    }
}
