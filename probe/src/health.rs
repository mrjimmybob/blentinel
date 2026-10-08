//! What the probe tells the hub about itself in each report (M4-02, D-73):
//! when it started, the reports it holds back, its clock against the hub's,
//! and its last problem.

use chrono::{DateTime, Duration, Utc};
use common::models::{ProbeHealth, ProbeProblem};

/// How long the probe keeps reporting a problem after it happened.
pub const PROBLEM_KEPT_HOURS: i64 = 24;

#[derive(Debug, Clone)]
pub struct Health {
    started_at: DateTime<Utc>,
    clock_offset_secs: Option<i64>,
    last_error: Option<ProbeProblem>,
}

impl Health {
    pub fn new(started_at: DateTime<Utc>) -> Self {
        Self {
            started_at,
            clock_offset_secs: None,
            last_error: None,
        }
    }

    /// Something went wrong at `now`; the newest problem replaces the last.
    pub fn problem(&mut self, message: impl Into<String>, now: DateTime<Utc>) {
        self.last_error = Some(ProbeProblem {
            at: now,
            message: message.into(),
        });
    }

    /// The hub answered at `hub_time` (by its clock), received at `now`.
    pub fn heard_from_hub(&mut self, hub_time: DateTime<Utc>, now: DateTime<Utc>) {
        self.clock_offset_secs = Some((now - hub_time).num_seconds());
    }

    /// The health for a report made at `now`, with `queued` reports waiting.
    /// A problem older than [`PROBLEM_KEPT_HOURS`] is forgotten.
    pub fn snapshot(&mut self, queued: i64, now: DateTime<Utc>) -> ProbeHealth {
        if self
            .last_error
            .as_ref()
            .is_some_and(|p| now - p.at > Duration::hours(PROBLEM_KEPT_HOURS))
        {
            self.last_error = None;
        }
        ProbeHealth {
            started_at: self.started_at,
            queued_reports: u32::try_from(queued.max(0)).unwrap_or(u32::MAX),
            clock_offset_secs: self.clock_offset_secs,
            last_error: self.last_error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_new_probe_reports_its_start_and_nothing_else() {
        let mut health = Health::new(at("2026-10-08T10:00:00Z"));
        let snapshot = health.snapshot(0, at("2026-10-08T10:00:05Z"));
        assert_eq!(snapshot.started_at, at("2026-10-08T10:00:00Z"));
        assert_eq!(snapshot.queued_reports, 0);
        assert_eq!(snapshot.clock_offset_secs, None);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn the_clock_offset_is_the_probe_minus_the_hub() {
        let mut health = Health::new(at("2026-10-08T10:00:00Z"));
        health.heard_from_hub(at("2026-10-08T10:00:00Z"), at("2026-10-08T10:01:30Z"));
        let now = at("2026-10-08T10:02:00Z");
        assert_eq!(health.snapshot(0, now).clock_offset_secs, Some(90));
        health.heard_from_hub(at("2026-10-08T10:05:00Z"), at("2026-10-08T10:04:58Z"));
        assert_eq!(health.snapshot(0, now).clock_offset_secs, Some(-2));
    }

    #[test]
    fn the_newest_problem_is_reported_for_a_day() {
        let mut health = Health::new(at("2026-10-08T10:00:00Z"));
        health.problem("first", at("2026-10-08T10:00:00Z"));
        health.problem("hub unreachable", at("2026-10-08T11:00:00Z"));
        let problem = health
            .snapshot(4, at("2026-10-09T10:59:00Z"))
            .last_error
            .unwrap();
        assert_eq!(problem.message, "hub unreachable");
        assert_eq!(
            health.snapshot(4, at("2026-10-09T11:01:00Z")).last_error,
            None
        );
    }

    #[test]
    fn the_queue_depth_never_goes_negative() {
        let mut health = Health::new(at("2026-10-08T10:00:00Z"));
        let now = at("2026-10-08T10:00:00Z");
        assert_eq!(health.snapshot(-1, now).queued_reports, 0);
        assert_eq!(health.snapshot(7, now).queued_reports, 7);
    }
}
