//! Thresholds and the status the hub gives each result (M3-01, proposal
//! 0005 §2.1, D-16, D-64).
//!
//! A check's warning and down limits come, limit by limit, from the lowest
//! level that sets them: **the check → its probe's site → the company → the
//! hub's default → the built-in default** ([`ResolvedLimits::resolve`]). Each
//! result is judged once, when it arrives ([`evaluate_report`]), and the
//! verdict is stored with it, so the dashboard, the probe page, uptime and
//! alerts agree. Changing a threshold applies from the next report on.

use common::models::StatusReport;

use crate::model::{
    CompanyScope, Evaluation, MetricLimits, ResolvedLimits, ThresholdLevel, ThresholdMetric,
    Thresholds, ThresholdsView,
};
use crate::service::scope;
use crate::storage::{CheckRecord, Storage};

#[derive(Debug, thiserror::Error)]
pub enum ThresholdError {
    /// Unknown, or outside the user's companies (deliberately the same).
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// The levels above a check: the site's, the company's and the hub's
/// thresholds, read once for a probe.
#[derive(Debug, Clone, Default)]
pub struct Levels {
    pub site: Thresholds,
    pub company: Thresholds,
    pub global: Thresholds,
}

impl Levels {
    /// The levels above a probe of `company_id` at `site_id`.
    pub async fn of(store: &dyn Storage, company_id: &str, site_id: &str) -> anyhow::Result<Self> {
        Ok(Self {
            site: store.site_thresholds(company_id, site_id).await?,
            company: store.company_thresholds(company_id).await?,
            global: store.threshold_defaults().await?.unwrap_or_default(),
        })
    }

    /// The limits in force for `metric` from the levels above a check.
    pub fn above_check(&self, metric: ThresholdMetric) -> ResolvedLimits {
        ResolvedLimits::resolve(
            metric,
            &[
                (ThresholdLevel::Site, self.site.get(metric)),
                (ThresholdLevel::Company, self.company.get(metric)),
                (ThresholdLevel::Global, self.global.get(metric)),
            ],
        )
    }

    /// A result of `check` as the hub judges it: by its own metric and its
    /// limits, and by what else its kind is judged by
    /// ([`ThresholdMetric::also_of`]); the worse verdict counts.
    pub fn judge(&self, result: &common::checks::CheckResult, check: &CheckRecord) -> Evaluation {
        let own = Evaluation::of(
            result,
            ThresholdMetric::of(&check.spec),
            self.for_check(check).as_ref(),
        );
        ThresholdMetric::also_of(&check.spec)
            .iter()
            .map(|metric| Evaluation::of(result, Some(*metric), Some(&self.above_check(*metric))))
            .fold(own, |worst, other| {
                if other.status.rank() < worst.status.rank() {
                    other
                } else {
                    worst
                }
            })
    }

    /// The limits in force for `check` (`None`: its kind has no thresholds).
    pub fn for_check(&self, check: &CheckRecord) -> Option<ResolvedLimits> {
        let metric = ThresholdMetric::of(&check.spec)?;
        Some(ResolvedLimits::resolve(
            metric,
            &[
                (ThresholdLevel::Check, check.limits),
                (ThresholdLevel::Site, self.site.get(metric)),
                (ThresholdLevel::Company, self.company.get(metric)),
                (ThresholdLevel::Global, self.global.get(metric)),
            ],
        ))
    }
}

/// The hub's verdict on each of a report's results, in their order. Its
/// results are of `checks` (ingest keeps only those).
pub async fn evaluate_report(
    store: &dyn Storage,
    report: &StatusReport,
    checks: &[CheckRecord],
) -> anyhow::Result<Vec<Evaluation>> {
    let levels = Levels::of(store, &report.company_id, &report.site).await?;
    let mut evaluations: Vec<Evaluation> = report
        .results
        .iter()
        .map(
            |result| match checks.iter().find(|c| c.id == result.check_id) {
                Some(check) => levels.judge(result, check),
                None => Evaluation::of(result, None, None),
            },
        )
        .collect();
    // Behind a check that is down, a check is unreachable (D-68).
    crate::service::dependencies::apply(&report.results, checks, &mut evaluations);
    Ok(evaluations)
}

/// What applies below a level that sets `own`, given the levels above it
/// (lowest first), metric by metric.
fn view(own: Thresholds, above: &[(ThresholdLevel, &Thresholds)]) -> ThresholdsView {
    let inherited = ThresholdMetric::ALL
        .into_iter()
        .map(|metric| {
            let levels: Vec<_> = above.iter().map(|(l, t)| (*l, t.get(metric))).collect();
            (metric, ResolvedLimits::resolve(metric, &levels))
        })
        .collect();
    ThresholdsView { own, inherited }
}

fn validate(thresholds: &Thresholds) -> Result<(), ThresholdError> {
    thresholds.validate().map_err(ThresholdError::Invalid)
}

/// The hub's default thresholds, for the admin page.
pub async fn global(store: &dyn Storage) -> anyhow::Result<ThresholdsView> {
    let own = store.threshold_defaults().await?.unwrap_or_default();
    Ok(view(own, &[]))
}

pub async fn set_global(
    store: &dyn Storage,
    thresholds: &Thresholds,
    by: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), ThresholdError> {
    validate(thresholds)?;
    store.set_threshold_defaults(thresholds, by, now).await?;
    Ok(())
}

async fn company_exists(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
) -> Result<crate::model::CompanyDetail, ThresholdError> {
    scope::require(scope, company_id).map_err(|_| ThresholdError::NotFound)?;
    store
        .company(company_id)
        .await?
        .ok_or(ThresholdError::NotFound)
}

/// A company's thresholds.
pub async fn company(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
) -> Result<ThresholdsView, ThresholdError> {
    company_exists(store, scope, company_id).await?;
    let own = store.company_thresholds(company_id).await?;
    let global = store.threshold_defaults().await?.unwrap_or_default();
    Ok(view(own, &[(ThresholdLevel::Global, &global)]))
}

pub async fn set_company(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
    thresholds: &Thresholds,
) -> Result<(), ThresholdError> {
    company_exists(store, scope, company_id).await?;
    validate(thresholds)?;
    if !store.set_company_thresholds(company_id, thresholds).await? {
        return Err(ThresholdError::NotFound);
    }
    Ok(())
}

async fn site_exists(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
    site_id: &str,
) -> Result<(), ThresholdError> {
    let detail = company_exists(store, scope, company_id).await?;
    match detail.sites.iter().any(|s| s.id == site_id) {
        true => Ok(()),
        false => Err(ThresholdError::NotFound),
    }
}

/// A site's thresholds.
pub async fn site(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
    site_id: &str,
) -> Result<ThresholdsView, ThresholdError> {
    site_exists(store, scope, company_id, site_id).await?;
    let own = store.site_thresholds(company_id, site_id).await?;
    let company = store.company_thresholds(company_id).await?;
    let global = store.threshold_defaults().await?.unwrap_or_default();
    Ok(view(
        own,
        &[
            (ThresholdLevel::Company, &company),
            (ThresholdLevel::Global, &global),
        ],
    ))
}

pub async fn set_site(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
    site_id: &str,
    thresholds: &Thresholds,
) -> Result<(), ThresholdError> {
    site_exists(store, scope, company_id, site_id).await?;
    validate(thresholds)?;
    if !store
        .set_site_thresholds(company_id, site_id, thresholds)
        .await?
    {
        return Err(ThresholdError::NotFound);
    }
    Ok(())
}

/// A check's own limits are for its kind's metric only.
pub fn validate_check_limits(
    spec: &common::checks::CheckSpec,
    limits: &MetricLimits,
) -> Result<(), String> {
    if limits.is_empty() {
        return Ok(());
    }
    match ThresholdMetric::of(spec) {
        Some(metric) => limits.validate(metric),
        None => Err(format!("a {} check has no thresholds", spec.kind())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Setting, Status};
    use common::checks::{CheckId, CheckResult, CheckSpec, Health, Metric, metric};

    fn at(value: f64) -> Option<Setting> {
        Some(Setting::At(value))
    }

    fn disk(usage: f64, health: Health) -> CheckResult {
        CheckResult {
            check_id: CheckId("c".into()),
            status: health,
            message: String::new(),
            latency_ms: None,
            metrics: vec![Metric {
                key: metric::USAGE.into(),
                value: usage,
                unit: "%".into(),
            }],
        }
    }

    fn check(spec: CheckSpec, limits: MetricLimits) -> CheckRecord {
        let now = chrono::Utc::now();
        CheckRecord {
            id: CheckId("c".into()),
            probe_id: "p".into(),
            company_id: "acme".into(),
            name: "c".into(),
            spec,
            enabled: true,
            alert_after: Default::default(),
            limits,
            depends_on: None,
            created_at: now,
            updated_at: now,
            updated_by: None,
        }
    }

    #[test]
    fn each_limit_comes_from_the_lowest_level_that_sets_it() {
        let mut levels = Levels::default();
        levels.company.disk = MetricLimits {
            warning: at(80.0),
            down: None,
        };
        levels.global.disk = MetricLimits {
            warning: at(70.0),
            down: at(99.0),
        };
        let own = MetricLimits {
            warning: None,
            down: Some(Setting::Off),
        };
        let resolved = levels
            .for_check(&check(CheckSpec::Disk { path: None }, own))
            .unwrap();
        assert_eq!(resolved.warning.value, Some(80.0));
        assert_eq!(resolved.warning.from, ThresholdLevel::Company);
        assert_eq!(resolved.down.value, None, "the check switched it off");
        assert_eq!(resolved.down.from, ThresholdLevel::Check);

        let nothing_set = Levels::default()
            .for_check(&check(
                CheckSpec::Disk { path: None },
                MetricLimits::default(),
            ))
            .unwrap();
        assert_eq!(
            (nothing_set.warning.value, nothing_set.down.value),
            (Some(90.0), Some(97.0))
        );
        assert_eq!(nothing_set.warning.from, ThresholdLevel::BuiltIn);
        assert!(
            Levels::default()
                .for_check(&check(CheckSpec::Uptime, MetricLimits::default()))
                .is_none()
        );
    }

    #[test]
    fn a_result_is_judged_against_its_limits() {
        let limits = Levels::default()
            .for_check(&check(
                CheckSpec::Disk { path: None },
                MetricLimits::default(),
            ))
            .unwrap();
        let judge = |usage, health| {
            Evaluation::of(
                &disk(usage, health),
                Some(ThresholdMetric::Disk),
                Some(&limits),
            )
        };
        assert_eq!(judge(50.0, Health::Up).status, Status::Up);
        let warning = judge(92.04, Health::Up);
        assert_eq!(warning.status, Status::Warning);
        assert_eq!(
            warning.reason.as_deref(),
            Some("disk usage 92% is over the warning limit (90%, from the built-in default)")
        );
        assert_eq!(judge(97.0, Health::Up).status, Status::Down, "at the limit");
        let failed = judge(10.0, Health::Down);
        assert_eq!((failed.status, failed.reason), (Status::Down, None));
    }

    #[test]
    fn latency_has_no_limits_until_someone_sets_them() {
        let mut result = disk(0.0, Health::Up);
        result.metrics.clear();
        result.latency_ms = Some(900);
        let ping = CheckSpec::ping("10.0.0.1");
        let none = Levels::default()
            .for_check(&check(ping.clone(), MetricLimits::default()))
            .unwrap();
        assert_eq!(
            Evaluation::of(&result, Some(ThresholdMetric::Latency), Some(&none)).status,
            Status::Up
        );
        let slow = Levels::default()
            .for_check(&check(
                ping,
                MetricLimits {
                    warning: at(200.0),
                    down: at(1000.0),
                },
            ))
            .unwrap();
        let judged = Evaluation::of(&result, Some(ThresholdMetric::Latency), Some(&slow));
        assert_eq!(judged.status, Status::Warning);
        assert!(judged.reason.unwrap().starts_with("latency 900 ms"));
    }

    #[test]
    fn a_certificate_is_judged_by_the_days_it_has_left() {
        let tls = CheckSpec::Tls {
            host: "example.com".into(),
            port: 443,
            verify: true,
        };
        let limits = Levels::default()
            .for_check(&check(tls, MetricLimits::default()))
            .unwrap();
        assert_eq!(
            (limits.warning.value, limits.down.value),
            (Some(21.0), Some(7.0))
        );
        let judge = |days_left: f64| {
            let mut result = disk(0.0, Health::Up);
            result.metrics = vec![Metric::new(metric::DAYS_LEFT, days_left, metric::DAYS)];
            Evaluation::of(&result, Some(ThresholdMetric::Tls), Some(&limits))
        };
        assert_eq!(judge(60.0).status, Status::Up);
        assert_eq!(judge(21.04).status, Status::Up, "just over the limit");
        let warning = judge(12.46);
        assert_eq!(warning.status, Status::Warning);
        assert_eq!(
            warning.reason.as_deref(),
            Some(
                "certificate expiry in 12.5 days is under the warning limit \
                 (21 days, from the built-in default)"
            )
        );
        assert_eq!(judge(21.0).status, Status::Warning, "at the limit");
        assert_eq!(judge(7.0).status, Status::Down, "at the limit");
        assert_eq!(judge(2.0).status, Status::Down);
    }

    #[test]
    fn a_ping_is_judged_by_its_latency_and_by_its_loss() {
        let mut levels = Levels::default();
        let ping = check(
            CheckSpec::ping("10.0.0.1"),
            MetricLimits {
                warning: at(100.0),
                down: None,
            },
        );
        let judge = |levels: &Levels, loss: f64, latency: u64| {
            let mut result = disk(0.0, Health::Up);
            result.metrics = vec![Metric::new(metric::LOSS, loss, metric::PERCENT)];
            result.latency_ms = Some(latency);
            levels.judge(&result, &ping)
        };
        assert_eq!(judge(&levels, 0.0, 20).status, Status::Up);
        assert_eq!(judge(&levels, 25.0, 20).status, Status::Up, "one of four");
        let lossy = judge(&levels, 50.0, 20);
        assert_eq!(lossy.status, Status::Warning);
        assert_eq!(
            lossy.reason.as_deref(),
            Some("packet loss 50% is over the warning limit (50%, from the built-in default)")
        );
        let slow = judge(&levels, 0.0, 250);
        assert_eq!(slow.status, Status::Warning);
        assert!(slow.reason.unwrap().starts_with("latency 250 ms"));

        // A site with a bad link is stricter, and the worse verdict counts.
        levels.site.loss = MetricLimits {
            warning: at(20.0),
            down: at(75.0),
        };
        assert_eq!(judge(&levels, 25.0, 20).status, Status::Warning);
        let bad = judge(&levels, 75.0, 250);
        assert_eq!(bad.status, Status::Down);
        assert!(bad.reason.unwrap().contains("from the site"));
    }

    #[test]
    fn a_file_is_judged_by_its_age() {
        let file = CheckSpec::File {
            path: "/var/backups".into(),
            pattern: "*.bak".into(),
            recursive: false,
            min_bytes: None,
        };
        let limits = Levels::default()
            .for_check(&check(file, MetricLimits::default()))
            .unwrap();
        let judge = |hours: f64| {
            let mut result = disk(0.0, Health::Up);
            result.metrics = vec![Metric::new(metric::AGE, hours, metric::HOURS)];
            Evaluation::of(&result, Some(ThresholdMetric::FileAge), Some(&limits))
        };
        assert_eq!(judge(8.0).status, Status::Up);
        let late = judge(30.25);
        assert_eq!(late.status, Status::Warning);
        assert_eq!(
            late.reason.as_deref(),
            Some("file age 30.3 h is over the warning limit (26 h, from the built-in default)")
        );
        assert_eq!(judge(50.0).status, Status::Down, "at the limit");
    }

    #[test]
    fn a_checks_limits_are_for_its_own_metric() {
        let limits = MetricLimits {
            warning: at(500.0),
            down: None,
        };
        let ping = CheckSpec::ping("10.0.0.1");
        assert!(validate_check_limits(&ping, &limits).is_ok(), "500 ms");
        assert!(
            validate_check_limits(&CheckSpec::Cpu, &limits).is_err(),
            "500 %"
        );
        assert!(validate_check_limits(&CheckSpec::Uptime, &limits).is_err());
        assert!(validate_check_limits(&CheckSpec::Uptime, &MetricLimits::default()).is_ok());
    }
}
