//! Maintenance windows and silences of any scope (M3-04, proposal 0005 §2.4,
//! D-65).
//!
//! A **window** is planned work: while it is open, its scope (the company, a
//! site, a probe or a check) sends no alerts, and its results are marked as
//! maintenance, so uptime leaves them out and the chart shades them. What is
//! still wrong when it closes alerts with the next report (`alerts::engine`
//! holds the news back instead of dropping it), and a probe that stopped
//! reporting during one alerts once it has closed.
//!
//! Its times are local to its time zone (a site's, usually), so "22:00" stays
//! 22:00 there across daylight saving: a start that falls in the hour the
//! clocks skip opens when that hour ends; one in the hour they repeat opens
//! the first time round.
//!
//! A **silence** is the ad-hoc kind: from now, for a while, until removed or
//! (a check's) until it recovers. Both are judged when a result arrives.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::model::{
    CompanyScope, CreateSilenceRequest, Evaluation, MaintenanceOverview, MaintenanceTarget,
    MaintenanceWindow, MonthDay, NewWindowRequest, Schedule, Scope, WindowInfo, parse_hhmm,
    parse_local_datetime,
};
use crate::service::scope;
use crate::storage::{CheckRecord, Storage};

#[derive(Debug, thiserror::Error)]
pub enum MaintenanceError {
    /// Unknown, or outside the user's companies (deliberately the same).
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// An IANA time zone by name (`Europe/Madrid`, `UTC`).
pub fn zone(name: &str) -> Result<Tz, String> {
    name.trim()
        .parse::<Tz>()
        .map_err(|_| format!("{name}: not a time zone (e.g. Europe/Madrid, America/New_York, UTC)"))
}

/// A local time in `tz` as an instant. In the hour the clocks skip, the
/// first instant after it; in the hour they repeat, the first time round.
pub fn to_utc(tz: Tz, local: NaiveDateTime) -> DateTime<Utc> {
    // In a gap, the first local minute that exists is the gap's end.
    (0..=24 * 60)
        .find_map(|minutes| {
            tz.from_local_datetime(&(local + Duration::minutes(minutes)))
                .earliest()
        })
        .map_or_else(
            || Utc.from_utc_datetime(&local),
            |at| at.with_timezone(&Utc),
        )
}

fn time_of(text: &str) -> NaiveTime {
    let (h, m) = parse_hhmm(text).unwrap_or((0, 0));
    NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN)
}

fn datetime_of(text: &str) -> Option<NaiveDateTime> {
    let (y, mo, d, h, mi) = parse_local_datetime(text)?;
    NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next = match month {
        12 => NaiveDate::from_ymd_opt(year + 1, 1, 1),
        m => NaiveDate::from_ymd_opt(year, m + 1, 1),
    };
    next.and_then(|n| n.pred_opt()).map_or(28, |d| d.day())
}

/// Does a recurring schedule open on local `date`?
fn opens_on(schedule: &Schedule, date: NaiveDate) -> bool {
    let weekday = date.weekday().number_from_monday();
    match schedule {
        Schedule::Once { .. } => false,
        Schedule::Weekly { days, .. } => days.contains(&weekday),
        Schedule::Monthly { on, .. } => match *on {
            // A day past the month's end: its last day.
            MonthDay::Date { day } => {
                date.day() == day.min(days_in_month(date.year(), date.month()))
            }
            MonthDay::Weekday {
                week,
                weekday: wanted,
            } => {
                if weekday != wanted {
                    return false;
                }
                let nth = (date.day() - 1) / 7 + 1;
                match week {
                    5 => date.day() + 7 > days_in_month(date.year(), date.month()),
                    w => nth == w,
                }
            }
        },
    }
}

fn start_and_minutes(schedule: &Schedule) -> (NaiveTime, i64) {
    match schedule {
        Schedule::Weekly { start, minutes, .. } | Schedule::Monthly { start, minutes, .. } => {
            (time_of(start), i64::from(*minutes))
        }
        Schedule::Once { .. } => (NaiveTime::MIN, 0),
    }
}

/// The opening of `window` that contains `now`, or else the next one
/// (`None`: a one-off that is over, or a bad time zone).
pub fn current_or_next(
    window: &MaintenanceWindow,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let tz = zone(&window.timezone).ok()?;
    if let Schedule::Once { start, end } = &window.schedule {
        let (start, end) = (
            to_utc(tz, datetime_of(start)?),
            to_utc(tz, datetime_of(end)?),
        );
        return (end > now).then_some((start, end));
    }
    let (time, minutes) = start_and_minutes(&window.schedule);
    let today = now.with_timezone(&tz).date_naive();
    // Openings that began up to `minutes` ago may still be open.
    let back = minutes / (24 * 60) + 1;
    let first = today - Duration::days(back);
    (0..=back + 400)
        .map(|offset| first + Duration::days(offset))
        .filter(|date| opens_on(&window.schedule, *date))
        .map(|date| {
            let start = to_utc(tz, date.and_time(time));
            (start, start + Duration::minutes(minutes))
        })
        .find(|(_, end)| *end > now)
}

/// Is `window` open at `now`?
pub fn is_open(window: &MaintenanceWindow, now: DateTime<Utc>) -> bool {
    current_or_next(window, now).is_some_and(|(start, _)| start <= now)
}

/// The scopes of `company_id`'s windows open at `now`.
pub async fn open_scopes(
    store: &dyn Storage,
    company_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<Scope>> {
    Ok(store
        .maintenance_windows(company_id)
        .await?
        .into_iter()
        .filter(|w| is_open(w, now))
        .map(|w| w.scope)
        .collect())
}

/// Mark the results of a report that came during a window (`evaluations`
/// in the order of its results).
pub async fn mark_report(
    store: &dyn Storage,
    report: &common::models::StatusReport,
    evaluations: &mut [Evaluation],
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let open = open_scopes(store, &report.company_id, now).await?;
    if open.is_empty() {
        return Ok(());
    }
    for (result, evaluation) in report.results.iter().zip(evaluations.iter_mut()) {
        evaluation.in_maintenance = open.iter().any(|scope| {
            scope.covers(
                &report.site,
                &report.probe_id,
                Some(result.check_id.as_str()),
            )
        });
    }
    Ok(())
}

/// Should a probe's "stopped reporting" alert wait? Yes while a window or a
/// silence covers the probe (or its site, or company).
pub async fn probe_is_quiet(
    store: &dyn Storage,
    probe_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let Some(record) = store.probe_record(probe_id).await? else {
        return Ok(false);
    };
    let (company, site) = (&record.company_id, &record.site_id);
    if store
        .is_silenced(company, site, probe_id, None, now)
        .await?
    {
        return Ok(true);
    }
    Ok(open_scopes(store, company, now)
        .await?
        .iter()
        .any(|scope| scope.covers(site, probe_id, None)))
}

// ---------------------------------------------------------------------------
// The maintenance page
// ---------------------------------------------------------------------------

/// The company's probes and checks, for naming a scope.
async fn targets(
    store: &dyn Storage,
    company_id: &str,
) -> anyhow::Result<Vec<(MaintenanceTarget, Vec<CheckRecord>)>> {
    let mut targets = Vec::new();
    for probe in store
        .probe_records(&CompanyScope::Only(
            std::iter::once(company_id.to_string()).collect(),
        ))
        .await?
        .into_iter()
        .filter(|p| p.company_id == company_id && p.state == "active")
    {
        let checks = store.probe_checks(&probe.id).await?;
        targets.push((
            MaintenanceTarget {
                probe_id: probe.id.clone(),
                probe_name: probe.name.clone(),
                site_id: probe.site_id.clone(),
                checks: checks
                    .iter()
                    .map(|c| (c.id.to_string(), c.name.clone()))
                    .collect(),
            },
            checks,
        ));
    }
    Ok(targets)
}

/// A scope in words, with the names the company page uses.
fn label(scope: &Scope, sites: &HashMap<String, String>, targets: &[MaintenanceTarget]) -> String {
    match scope {
        Scope::Company => "the whole company".into(),
        Scope::Site { site_id } => format!(
            "site {}",
            sites
                .get(site_id)
                .cloned()
                .unwrap_or_else(|| site_id.clone())
        ),
        Scope::Probe { probe_id } => format!(
            "probe {}",
            targets
                .iter()
                .find(|t| &t.probe_id == probe_id)
                .map_or_else(
                    || crate::model::short_probe_id(probe_id).to_string(),
                    |t| t.probe_name.clone()
                )
        ),
        Scope::Check { check_id } => targets
            .iter()
            .find_map(|t| {
                t.checks
                    .iter()
                    .find(|(id, _)| id == check_id)
                    .map(|(_, name)| format!("check {name} on {}", t.probe_name))
            })
            .unwrap_or_else(|| "a deleted check".into()),
    }
}

async fn company_in_scope(
    store: &dyn Storage,
    user_scope: &CompanyScope,
    company_id: &str,
) -> Result<crate::model::CompanyDetail, MaintenanceError> {
    scope::require(user_scope, company_id).map_err(|_| MaintenanceError::NotFound)?;
    store
        .company(company_id)
        .await?
        .ok_or(MaintenanceError::NotFound)
}

/// A company's windows and silences, and what they can cover.
pub async fn overview(
    store: &dyn Storage,
    user_scope: &CompanyScope,
    company_id: &str,
    now: DateTime<Utc>,
) -> Result<MaintenanceOverview, MaintenanceError> {
    let detail = company_in_scope(store, user_scope, company_id).await?;
    let sites: HashMap<String, String> = detail
        .sites
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    let probes: Vec<MaintenanceTarget> = targets(store, company_id)
        .await?
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let windows = store
        .maintenance_windows(company_id)
        .await?
        .into_iter()
        .map(|window| {
            let opening = current_or_next(&window, now);
            let next = opening.and_then(|(start, end)| {
                let tz = zone(&window.timezone).ok()?;
                let (s, e) = (start.with_timezone(&tz), end.with_timezone(&tz));
                let end_text = match s.date_naive() == e.date_naive() {
                    true => e.format("%H:%M").to_string(),
                    false => e.format("%Y-%m-%d %H:%M").to_string(),
                };
                Some(format!(
                    "{} – {end_text} ({})",
                    s.format("%Y-%m-%d %H:%M"),
                    window.timezone
                ))
            });
            WindowInfo {
                scope_label: label(&window.scope, &sites, &probes),
                schedule_text: window.schedule.describe(),
                active: opening.is_some_and(|(start, _)| start <= now),
                next,
                window,
            }
        })
        .collect();
    let mut silences = store.active_silences(now).await?;
    silences.retain(|s| s.company_id == company_id);
    Ok(MaintenanceOverview {
        windows,
        silences,
        sites: detail.sites,
        probes,
    })
}

/// The scope names something of the company.
async fn check_scope(
    store: &dyn Storage,
    company: &crate::model::CompanyDetail,
    scope: &Scope,
) -> Result<(), MaintenanceError> {
    let company_id = &company.company.id;
    let exists = match scope {
        Scope::Company => true,
        Scope::Site { site_id } => company.sites.iter().any(|s| &s.id == site_id),
        Scope::Probe { probe_id } => store
            .probe_record(probe_id)
            .await?
            .is_some_and(|p| &p.company_id == company_id),
        Scope::Check { check_id } => store
            .check(&common::checks::CheckId(check_id.clone()))
            .await?
            .is_some_and(|c| &c.company_id == company_id),
    };
    match exists {
        true => Ok(()),
        false => Err(MaintenanceError::Invalid(format!(
            "no such {} in this company",
            scope.kind()
        ))),
    }
}

/// Add a window to a company of the user's.
pub async fn create_window(
    store: &dyn Storage,
    user_scope: &CompanyScope,
    company_id: &str,
    request: &NewWindowRequest,
    by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<MaintenanceWindow, MaintenanceError> {
    let company = company_in_scope(store, user_scope, company_id).await?;
    request.validate().map_err(MaintenanceError::Invalid)?;
    zone(&request.timezone).map_err(MaintenanceError::Invalid)?;
    check_scope(store, &company, &request.scope).await?;
    let id = store
        .create_maintenance_window(company_id, request, by, now)
        .await?;
    store
        .maintenance_window(id)
        .await?
        .ok_or(MaintenanceError::NotFound)
}

/// Remove a window of a company of the user's; returns it.
pub async fn delete_window(
    store: &dyn Storage,
    user_scope: &CompanyScope,
    id: i64,
) -> Result<MaintenanceWindow, MaintenanceError> {
    let window = store
        .maintenance_window(id)
        .await?
        .ok_or(MaintenanceError::NotFound)?;
    scope::require(user_scope, &window.company_id).map_err(|_| MaintenanceError::NotFound)?;
    store.delete_maintenance_window(id).await?;
    Ok(window)
}

/// Silence a scope of a company of the user's, now; returns the silence's ID.
pub async fn silence(
    store: &dyn Storage,
    user_scope: &CompanyScope,
    request: &CreateSilenceRequest,
    now: DateTime<Utc>,
) -> Result<i64, MaintenanceError> {
    let company = company_in_scope(store, user_scope, &request.company_id).await?;
    if request.reason.trim().is_empty() {
        return Err(MaintenanceError::Invalid("say why".into()));
    }
    check_scope(store, &company, &request.scope).await?;
    let expires_at = request
        .duration_hours
        .map(|hours| now + Duration::hours(i64::from(hours)));
    Ok(store
        .create_silence(
            &request.company_id,
            &request.scope,
            request.reason.trim(),
            expires_at,
            now,
        )
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(timezone: &str, schedule: Schedule) -> MaintenanceWindow {
        MaintenanceWindow {
            id: 1,
            company_id: "acme".into(),
            scope: Scope::Company,
            reason: "patching".into(),
            timezone: timezone.into(),
            schedule,
            created_by: None,
            created_at: String::new(),
        }
    }

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn weekly(days: &[u32], start: &str, minutes: u32) -> Schedule {
        Schedule::Weekly {
            days: days.to_vec(),
            start: start.into(),
            minutes,
        }
    }

    #[test]
    fn a_weekly_window_follows_the_sites_local_time() {
        // Tuesdays 22:00 for 4 h in Madrid (UTC+2 in October).
        let w = window("Europe/Madrid", weekly(&[2], "22:00", 240));
        assert!(!is_open(&w, utc("2026-10-06T19:59:00Z")), "21:59 local");
        assert!(is_open(&w, utc("2026-10-06T20:00:00Z")), "22:00 local");
        assert!(
            is_open(&w, utc("2026-10-06T23:59:00Z")),
            "past midnight, still open"
        );
        assert!(
            !is_open(&w, utc("2026-10-07T00:00:00Z")),
            "02:00 local: closed"
        );
        assert_eq!(
            current_or_next(&w, utc("2026-10-07T01:00:00Z")),
            Some((utc("2026-10-13T20:00:00Z"), utc("2026-10-14T00:00:00Z"))),
            "next Tuesday"
        );
    }

    #[test]
    fn local_time_holds_across_daylight_saving() {
        // Sundays 01:00 for 1 h in Madrid: UTC+2 before 25 October, UTC+1 after.
        let w = window("Europe/Madrid", weekly(&[7], "01:00", 60));
        assert_eq!(
            current_or_next(&w, utc("2026-10-19T00:00:00Z")).unwrap().0,
            utc("2026-10-24T23:00:00Z")
        );
        assert_eq!(
            current_or_next(&w, utc("2026-10-26T00:00:00Z")).unwrap().0,
            utc("2026-11-01T00:00:00Z")
        );
        // 02:30 doesn't exist on 29 March 2026 (02:00 → 03:00): it opens at 03:00.
        let gap = window("Europe/Madrid", weekly(&[7], "02:30", 60));
        assert_eq!(
            current_or_next(&gap, utc("2026-03-28T00:00:00Z"))
                .unwrap()
                .0,
            utc("2026-03-29T01:00:00Z")
        );
        // 02:30 happens twice on 25 October 2026: the first time round.
        assert_eq!(
            current_or_next(&gap, utc("2026-10-24T00:00:00Z"))
                .unwrap()
                .0,
            utc("2026-10-25T00:30:00Z")
        );
    }

    #[test]
    fn monthly_windows_by_date_or_weekday() {
        let patch_tuesday = Schedule::Monthly {
            on: MonthDay::Weekday {
                week: 2,
                weekday: 2,
            },
            start: "22:00".into(),
            minutes: 180,
        };
        let w = window("UTC", patch_tuesday);
        assert_eq!(
            current_or_next(&w, utc("2026-10-01T00:00:00Z")).unwrap().0,
            utc("2026-10-13T22:00:00Z"),
            "the 2nd Tuesday of October 2026"
        );
        let last_friday = Schedule::Monthly {
            on: MonthDay::Weekday {
                week: 5,
                weekday: 5,
            },
            start: "18:00".into(),
            minutes: 60,
        };
        assert_eq!(
            current_or_next(&window("UTC", last_friday), utc("2026-10-01T00:00:00Z"))
                .unwrap()
                .0,
            utc("2026-10-30T18:00:00Z")
        );
        let the_31st = Schedule::Monthly {
            on: MonthDay::Date { day: 31 },
            start: "03:00".into(),
            minutes: 60,
        };
        assert_eq!(
            current_or_next(&window("UTC", the_31st), utc("2026-11-01T00:00:00Z"))
                .unwrap()
                .0,
            utc("2026-11-30T03:00:00Z"),
            "a short month: its last day"
        );
    }

    #[test]
    fn a_one_off_window_opens_once() {
        let w = window(
            "America/New_York",
            Schedule::Once {
                start: "2026-10-10T09:00".into(),
                end: "2026-10-10T17:00".into(),
            },
        );
        assert!(!is_open(&w, utc("2026-10-10T12:59:00Z")));
        assert!(is_open(&w, utc("2026-10-10T13:00:00Z")), "09:00 EDT");
        assert!(!is_open(&w, utc("2026-10-10T21:00:00Z")));
        assert_eq!(
            current_or_next(&w, utc("2026-10-11T00:00:00Z")),
            None,
            "over"
        );
    }

    #[test]
    fn time_zones_are_checked() {
        assert!(zone("Europe/Madrid").is_ok());
        assert!(zone("UTC").is_ok());
        assert!(
            zone("Mars/Olympus")
                .unwrap_err()
                .contains("not a time zone")
        );
        let bad = window("Mars/Olympus", weekly(&[1], "00:00", 60));
        assert!(!is_open(&bad, utc("2026-10-05T00:30:00Z")));
    }

    #[test]
    fn scopes_cover_what_they_name() {
        let site = Scope::Site {
            site_id: "hq".into(),
        };
        assert!(site.covers("hq", "p1", Some("c1")));
        assert!(site.covers("hq", "p1", None), "the probe itself");
        assert!(!site.covers("branch", "p1", Some("c1")));
        let check = Scope::Check {
            check_id: "c1".into(),
        };
        assert!(check.covers("hq", "p1", Some("c1")));
        assert!(!check.covers("hq", "p1", None), "not the probe's own alert");
        assert!(Scope::Company.covers("x", "y", None));
    }
}
