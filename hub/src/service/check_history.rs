//! A check over a period (M6-02, proposal 0007 §3.1): its status band, metric
//! charts, availability, incidents and MTTR, and its newest raw results.
//!
//! Everything but the raw results is read from the rollups (D-111), at the
//! finest width still kept for the period ([`width_for`]), so a year shows as
//! well as a day, long after the raw results are gone.

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};

use crate::model::{
    CheckHistory, CheckResultEntry, CompanyScope, HistoryRange, IncidentSpan, MetricSeries,
    SeriesPoint, StatusBucket, StatusCounts,
};
use crate::service::checks::scoped_check;
use crate::service::history::{HistoryError, width_for};
use crate::service::probes::ProbeError;
use crate::storage::{CheckBucket, MetricBucket, Storage, StoredIncident, StoredResult};

/// How many raw results the page lists.
pub const RECENT_RESULTS: u32 = 50;

/// The key of the response-time series, the first chart.
pub const LATENCY_KEY: &str = "response time";

/// The longest period that can be asked for.
pub const MAX_DAYS: i64 = 3653;

/// The period a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// Ending now.
    Last(HistoryRange),
    /// Whole UTC days, both included.
    Days { from: NaiveDate, to: NaiveDate },
}

impl Period {
    /// From `?range=`, or `?from=&to=` (`YYYY-MM-DD`). Neither: the last 24 hours.
    pub fn parse(
        range: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Self, HistoryError> {
        let invalid = |text: String| HistoryError::InvalidRange(text);
        let day = |text: &str| {
            NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
                .map_err(|_| invalid(format!("{text}: not a date (YYYY-MM-DD)")))
        };
        match (range, from, to) {
            (None, None, None) => Ok(Self::Last(HistoryRange::Day)),
            (Some(range), None, None) => HistoryRange::parse(range)
                .map(Self::Last)
                .ok_or_else(|| invalid(format!("{range}: the range is 24h, 7d, 30d or 1y"))),
            (None, Some(from), Some(to)) => Ok(Self::Days {
                from: day(from)?,
                to: day(to)?,
            }),
            _ => Err(invalid("give a range, or both from and to".into())),
        }
    }

    /// `[from, to)` as of `now`; a period of days ends now at the latest.
    pub fn bounds(
        self,
        now: DateTime<Utc>,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), HistoryError> {
        match self {
            Self::Last(range) => Ok((now - Duration::days(range.days()), now)),
            Self::Days { from, to } => {
                let invalid = |text: &str| Err(HistoryError::InvalidRange(text.into()));
                if from > to {
                    return invalid("the first day is after the last");
                }
                if (to - from).num_days() >= MAX_DAYS {
                    return invalid("a period is at most 10 years");
                }
                let midnight = |d: NaiveDate| Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN));
                let (from, to) = (midnight(from), midnight(to) + Duration::days(1));
                if from >= now {
                    return invalid("the period hasn't started yet");
                }
                Ok((from, to.min(now)))
            }
        }
    }
}

/// A check of one of the user's companies over `period`.
pub async fn check_history(
    store: &dyn Storage,
    scope: &CompanyScope,
    check_id: &str,
    period: Period,
    now: DateTime<Utc>,
) -> Result<CheckHistory, HistoryError> {
    let check = scoped_check(store, scope, check_id)
        .await
        .map_err(|e| match e {
            ProbeError::NotFound => HistoryError::NotFound,
            ProbeError::Storage(e) => HistoryError::Failed(e),
            other => HistoryError::Failed(anyhow::anyhow!(other)),
        })?;
    let (from, to) = period.bounds(now)?;

    let probe = store.probe_record(&check.probe_id).await?;
    let company = store.company(&check.company_id).await?;
    let site_name = company.as_ref().and_then(|c| {
        let site_id = &probe.as_ref()?.site_id;
        c.sites
            .iter()
            .find(|s| &s.id == site_id)
            .map(|s| s.name.clone())
    });

    let retention = store.history_retention().await?;
    let width = width_for(&retention, from, to, now);
    let buckets = store
        .check_history(&check.company_id, check_id, width, from, to)
        .await?;
    let metrics = store
        .metric_history(&check.company_id, check_id, width, from, to)
        .await?;
    let incidents = store.check_incidents(check_id, from, to).await?;
    let recent = store.check_results(check_id, RECENT_RESULTS).await?;

    let totals = totals(&buckets);
    let (incident_count, mttr_secs) = incident_stats(&incidents, from, to);
    Ok(CheckHistory {
        check_id: check.id.0.clone(),
        name: check.name.clone(),
        kind: check.spec.kind().to_string(),
        target: check.spec.target(),
        enabled: check.enabled,
        probe_name: probe
            .as_ref()
            .map_or_else(|| check.probe_id.clone(), |p| p.name.clone()),
        probe_id: check.probe_id.clone(),
        company_name: company
            .as_ref()
            .map_or_else(|| check.company_id.clone(), |c| c.company.name.clone()),
        company_id: check.company_id.clone(),
        site_name: site_name.unwrap_or_default(),
        from: minute(from),
        to: minute(to),
        from_unix: from.timestamp(),
        to_unix: to.timestamp(),
        bucket_secs: width.seconds(),
        availability_pct: totals.availability_pct(),
        totals,
        incident_count,
        mttr_secs,
        buckets: buckets
            .iter()
            .map(|b| StatusBucket {
                start_unix: b.start.timestamp(),
                start: minute(b.start),
                counts: b.counts,
            })
            .collect(),
        series: series(&buckets, &metrics),
        incidents: incidents.iter().map(span).collect(),
        recent: recent.into_iter().map(entry).collect(),
    })
}

fn minute(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

/// Every result in the buckets.
pub fn totals(buckets: &[CheckBucket]) -> StatusCounts {
    let mut totals = StatusCounts::default();
    for bucket in buckets {
        totals.add(&bucket.counts);
    }
    totals
}

/// How many incidents opened in `[from, to)`, and the average length of
/// those that closed in it (MTTR, in seconds).
pub fn incident_stats(
    incidents: &[StoredIncident],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> (u32, Option<i64>) {
    let within = |at: DateTime<Utc>| from <= at && at < to;
    let opened = incidents.iter().filter(|i| within(i.opened_at)).count();
    let lengths: Vec<i64> = incidents
        .iter()
        .filter_map(|i| {
            let closed = i.closed_at.filter(|at| within(*at))?;
            Some((closed - i.opened_at).num_seconds())
        })
        .collect();
    let mttr = (!lengths.is_empty()).then(|| lengths.iter().sum::<i64>() / lengths.len() as i64);
    (u32::try_from(opened).unwrap_or(u32::MAX), mttr)
}

/// Response time (when the check measures it), then each metric by key.
pub fn series(buckets: &[CheckBucket], metrics: &[MetricBucket]) -> Vec<MetricSeries> {
    let point = |start: DateTime<Utc>, min: f64, max: f64, avg: f64| SeriesPoint {
        start_unix: start.timestamp(),
        start: minute(start),
        min,
        max,
        avg,
    };
    let latency: Vec<SeriesPoint> = buckets
        .iter()
        .filter_map(|b| {
            Some(point(
                b.start,
                b.latency_min_ms? as f64,
                b.latency_max_ms? as f64,
                b.latency_avg_ms?,
            ))
        })
        .collect();
    let mut all = Vec::new();
    if !latency.is_empty() {
        all.push(MetricSeries {
            key: LATENCY_KEY.into(),
            unit: "ms".into(),
            points: latency,
        });
    }
    // Storage gives them by key, then oldest first.
    for m in metrics {
        let p = point(m.start, m.min, m.max, m.avg);
        match all.last_mut() {
            Some(s) if s.key == m.key && s.key != LATENCY_KEY => {
                s.unit.clone_from(&m.unit);
                s.points.push(p);
            }
            _ => all.push(MetricSeries {
                key: m.key.clone(),
                unit: m.unit.clone(),
                points: vec![p],
            }),
        }
    }
    all
}

fn span(incident: &StoredIncident) -> IncidentSpan {
    IncidentSpan {
        id: incident.id,
        severity: incident.severity,
        opened_at: minute(incident.opened_at),
        opened_unix: incident.opened_at.timestamp(),
        closed_at: incident.closed_at.map(minute),
        closed_unix: incident.closed_at.map(|at| at.timestamp()),
    }
}

fn entry(result: StoredResult) -> CheckResultEntry {
    CheckResultEntry {
        at: result.at.format("%Y-%m-%d %H:%M:%S").to_string(),
        status: result.status,
        reason: result.reason,
        message: result.message,
        latency_ms: result.latency_ms,
        in_maintenance: result.in_maintenance,
        metrics: result.metrics,
    }
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
    fn a_period_is_a_range_or_whole_days_and_never_ends_after_now() {
        let now = t("2026-03-10T12:00:00Z");
        let last = Period::parse(Some("7d"), None, None).unwrap();
        assert_eq!(last.bounds(now).unwrap(), (t("2026-03-03T12:00:00Z"), now));
        assert_eq!(
            Period::parse(None, None, None).unwrap(),
            Period::Last(HistoryRange::Day)
        );

        let days = Period::parse(None, Some("2026-03-01"), Some("2026-03-02")).unwrap();
        assert_eq!(
            days.bounds(now).unwrap(),
            (t("2026-03-01T00:00:00Z"), t("2026-03-03T00:00:00Z"))
        );
        let today = Period::parse(None, Some("2026-03-09"), Some("2026-03-10")).unwrap();
        assert_eq!(today.bounds(now).unwrap().1, now, "ends now");

        for (range, from, to) in [
            (Some("2y"), None, None),
            (None, Some("2026-03-01"), None),
            (Some("7d"), Some("2026-03-01"), Some("2026-03-02")),
            (None, Some("1 March"), Some("2026-03-02")),
        ] {
            assert!(
                Period::parse(range, from, to).is_err(),
                "{range:?} {from:?} {to:?}"
            );
        }
        for (from, to) in [
            ("2026-03-05", "2026-03-01"),
            ("2026-03-11", "2026-03-12"),
            ("2010-01-01", "2026-03-01"),
        ] {
            let period = Period::parse(None, Some(from), Some(to)).unwrap();
            assert!(period.bounds(now).is_err(), "{from} {to}");
        }
    }

    #[test]
    fn incidents_count_where_they_open_and_mttr_where_they_close() {
        let (from, to) = (t("2026-03-01T00:00:00Z"), t("2026-03-02T00:00:00Z"));
        let incidents = [
            // Opened before, closed within: MTTR only (2 hours).
            incident("2026-02-28T23:00:00Z", Some("2026-03-01T01:00:00Z")),
            // Opened and closed within: both (10 minutes).
            incident("2026-03-01T10:00:00Z", Some("2026-03-01T10:10:00Z")),
            // Opened within, still open: counted, no MTTR.
            incident("2026-03-01T20:00:00Z", None),
            // Opened within, closed after: counted, no MTTR.
            incident("2026-03-01T22:00:00Z", Some("2026-03-02T02:00:00Z")),
        ];
        let (count, mttr) = incident_stats(&incidents, from, to);
        assert_eq!(count, 3);
        assert_eq!(mttr, Some((7200 + 600) / 2));
        assert_eq!(incident_stats(&[], from, to), (0, None));
    }

    #[test]
    fn response_time_comes_first_then_each_metric_in_its_own_series() {
        let start = t("2026-03-01T00:00:00Z");
        let bucket = |start, latency: Option<(i64, i64, f64)>| CheckBucket {
            start,
            counts: StatusCounts::default(),
            latency_min_ms: latency.map(|l| l.0),
            latency_max_ms: latency.map(|l| l.1),
            latency_avg_ms: latency.map(|l| l.2),
        };
        let metric = |key: &str, start, avg| MetricBucket {
            start,
            key: key.into(),
            unit: "%".into(),
            min: avg,
            max: avg,
            avg,
        };
        let later = start + Duration::hours(1);
        let buckets = [bucket(start, Some((10, 30, 20.0))), bucket(later, None)];
        let metrics = [
            metric("cpu", start, 5.0),
            metric("cpu", later, 7.0),
            metric("memory", start, 50.0),
        ];
        let all = series(&buckets, &metrics);
        let keys: Vec<&str> = all.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, [LATENCY_KEY, "cpu", "memory"]);
        assert_eq!(all[0].unit, "ms");
        assert_eq!(all[0].points.len(), 1, "only buckets with a latency");
        assert_eq!(all[1].points.len(), 2);
        assert_eq!(all[1].points[1].avg, 7.0);

        let none = series(&[bucket(start, None)], &[]);
        assert!(none.is_empty(), "no response time, no metrics");
    }
}
