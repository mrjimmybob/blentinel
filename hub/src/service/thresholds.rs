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
        let verdict = ThresholdMetric::also_of(&check.spec)
            .iter()
            .map(|metric| Evaluation::of(result, Some(*metric), Some(&self.above_check(*metric))))
            .fold(own, |worst, other| {
                if other.status.rank() < worst.status.rank() {
                    other
                } else {
                    worst
                }
            });
        let mut verdict = verdict;
        // An `snmp` check's values have limits of their own (D-87).
        if let common::checks::CheckSpec::Snmp { oids, .. } = &check.spec
            && let Some(values) = snmp_verdict(result, oids)
            && values.status.rank() < verdict.status.rank()
        {
            verdict = values;
        }
        // An `updates` check is judged by its own limits, in days (D-89).
        if let common::checks::CheckSpec::Updates {
            security_days,
            reboot_days,
            stale_days,
        } = &check.spec
            && let Some(waiting) =
                updates_verdict(result, *security_days, *reboot_days, *stale_days)
        {
            verdict = waiting;
        }
        // An `eventlog` check is judged by its own count (D-91).
        if let common::checks::CheckSpec::Eventlog {
            count,
            wanted,
            warning,
            ..
        } = &check.spec
            && let Some(events) = eventlog_verdict(result, *count, *wanted, *warning)
        {
            verdict = events;
        }
        // A `smart` check's warning signs are a Warning (D-93), a UPS's
        // (`nut`, D-97), and a virtual machine's (`hyperv`, D-98).
        if matches!(
            check.spec,
            common::checks::CheckSpec::Smart { .. }
                | common::checks::CheckSpec::Nut { .. }
                | common::checks::CheckSpec::Hyperv { .. }
        ) && let Some(signs) = smart_verdict(result)
        {
            verdict = signs;
        }
        // A threshold's reason is shown instead of the probe's message; of
        // several disks, only the message says which one it is (D-81), and
        // of several sensors (D-94); of a clock, which way it is off and
        // from which server (D-96).
        if matches!(
            check.spec,
            common::checks::CheckSpec::Disks { .. }
                | common::checks::CheckSpec::Temperature { .. }
                | common::checks::CheckSpec::Ntp { .. }
        ) && let Some(reason) = &mut verdict.reason
        {
            *reason = format!("{reason}: {}", result.message);
        }
        verdict
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

/// The value of an `snmp` check that the number called `key` is of: the one
/// by that name, else the table with a row by that name.
fn value_of<'a>(
    oids: &'a [common::checks::SnmpOid],
    key: &str,
) -> Option<&'a common::checks::SnmpOid> {
    oids.iter()
        .filter(|value| value.owns(key))
        .max_by_key(|value| value.label().len())
}

/// An `snmp` check's numbers against the limits of its values (M5-09,
/// D-87): the worst one that is past a limit, and how many more are.
/// `None`: none is, or the probe already says the check is down.
fn snmp_verdict(
    result: &common::checks::CheckResult,
    oids: &[common::checks::SnmpOid],
) -> Option<Evaluation> {
    if result.status == common::checks::Health::Down {
        return None;
    }
    let mut worst: Option<(crate::model::Status, String)> = None;
    let mut past = 0;
    for metric in &result.metrics {
        let Some(value) = value_of(oids, &metric.key) else {
            continue;
        };
        let reached = [
            (crate::model::Status::Down, value.down, "down"),
            (crate::model::Status::Warning, value.warn, "warning"),
        ]
        .into_iter()
        .find_map(|(status, limit, name)| {
            let limit = limit.filter(|limit| limit.reached(metric.value))?;
            Some((status, limit, name))
        });
        let Some((status, limit, name)) = reached else {
            continue;
        };
        past += 1;
        if worst
            .as_ref()
            .is_none_or(|(so_far, _)| status.rank() < so_far.rank())
        {
            let unit = match metric.unit.as_str() {
                "" => String::new(),
                unit => format!(" {unit}"),
            };
            worst = Some((
                status,
                format!(
                    "{} {}{unit} is {} the {name} limit ({}{unit})",
                    metric.key,
                    (metric.value * 10.0).round() / 10.0,
                    if limit.is_under() { "under" } else { "over" },
                    limit.at(),
                ),
            ));
        }
    }
    let (status, mut reason) = worst?;
    if past > 1 {
        reason.push_str(&format!(", and {} more past their limits", past - 1));
    }
    Some(Evaluation {
        status,
        reason: Some(reason),
        in_maintenance: false,
        behind: None,
    })
}

/// A `smart` check's warning signs (M5-16, D-93): a Warning when a drive
/// shows one (bad sectors, an attribute past its limit, a worn-out SSD),
/// in the probe's words, which name the drive. `None`: no sign, or the
/// probe says the check is down (a drive is failing: its Down stands).
/// The same for a `nut` check's UPS (D-97): on battery, a battery to
/// replace, an overload; and for a `hyperv` check's machines (D-98).
fn smart_verdict(result: &common::checks::CheckResult) -> Option<Evaluation> {
    if result.status == common::checks::Health::Down {
        return None;
    }
    let signs = result.metric(common::checks::metric::WARNINGS)?.value;
    (signs >= 1.0).then(|| Evaluation {
        status: crate::model::Status::Warning,
        reason: Some(result.message.clone()),
        in_maintenance: false,
        behind: None,
    })
}

/// An `eventlog` check's count against what the check asks (M5-14, D-91):
/// `count` events are trouble, or (`wanted`) fewer than `count` are. Down,
/// or a Warning if the check says so. `None`: no trouble, or the probe
/// couldn't read the log (its own Down stands).
fn eventlog_verdict(
    result: &common::checks::CheckResult,
    count: u32,
    wanted: bool,
    warning: bool,
) -> Option<Evaluation> {
    if result.status == common::checks::Health::Down {
        return None;
    }
    let events = result.metric(common::checks::metric::EVENTS)?.value;
    let count = f64::from(count);
    let trouble = if wanted {
        events < count
    } else {
        events >= count
    };
    if !trouble {
        return None;
    }
    // The probe's words say what was found; only "too few" needs more.
    let reason = if wanted && events >= 1.0 {
        format!("Fewer than the {count} expected: {}", result.message)
    } else {
        result.message.clone()
    };
    Some(Evaluation {
        status: if warning {
            crate::model::Status::Warning
        } else {
            crate::model::Status::Down
        },
        reason: Some(reason),
        in_maintenance: false,
        behind: None,
    })
}

/// An `updates` check's numbers against its limits (M5-11, D-89): a
/// Warning for security updates, or a restart, that have waited as long as
/// their limit, and for a system that hasn't looked for updates for as long
/// as its. `None`: nothing has, or the probe says the check is down. Never
/// Down: nothing is broken while updates wait.
fn updates_verdict(
    result: &common::checks::CheckResult,
    security_days: Option<u32>,
    reboot_days: Option<u32>,
    stale_days: Option<u32>,
) -> Option<Evaluation> {
    use common::checks::metric;
    if result.status == common::checks::Health::Down {
        return None;
    }
    let value = |key: &str| result.metric(key).map(|m| m.value);
    // Days as a person writes them: `9`, `9.5`.
    let days = |days: f64| {
        let days = (days * 10.0).round() / 10.0;
        if days == 1.0 {
            "1 day".to_string()
        } else {
            format!("{days} days")
        }
    };
    let reached = |limit: Option<u32>, waited: f64| limit.filter(|l| waited >= f64::from(*l));
    let mut reasons = Vec::new();

    let security = value(metric::SECURITY).unwrap_or(0.0);
    let waited = value(metric::SECURITY_DAYS).unwrap_or(0.0);
    if security >= 1.0
        && let Some(limit) = reached(security_days, waited)
    {
        let what = if security == 1.0 {
            "1 security update".to_string()
        } else {
            format!("{security} security updates")
        };
        reasons.push(match limit {
            0 if security == 1.0 => format!("{what} waits"),
            0 => format!("{what} wait"),
            _ if security == 1.0 => {
                format!("{what} has waited {} (limit {limit})", days(waited))
            }
            _ => format!("{what} have waited {} (limit {limit})", days(waited)),
        });
    }

    let waited = value(metric::REBOOT_DAYS).unwrap_or(0.0);
    if value(metric::REBOOT) == Some(1.0)
        && let Some(limit) = reached(reboot_days, waited)
    {
        reasons.push(match limit {
            0 => "a restart is needed to finish updates".to_string(),
            _ => format!(
                "a restart has been needed for {} (limit {limit})",
                days(waited)
            ),
        });
    }

    if let Some(looked) = value(metric::LOOKED_DAYS)
        && let Some(limit) = reached(stale_days, looked)
    {
        reasons.push(format!(
            "the system last looked for updates {} ago (limit {limit})",
            days(looked)
        ));
    }

    if reasons.is_empty() {
        return None;
    }
    let mut reason = reasons.join("; ");
    if let Some(first) = reason.get(..1) {
        reason = format!("{}{}", first.to_uppercase(), &reason[1..]);
    }
    Some(Evaluation {
        status: crate::model::Status::Warning,
        reason: Some(reason),
        in_maintenance: false,
        behind: None,
    })
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
            secrets: Default::default(),
            created_at: now,
            updated_at: now,
            updated_by: None,
        }
    }

    #[test]
    fn an_snmp_checks_values_are_judged_by_their_own_limits() {
        let oids = common::snmp::parse_values(
            "Supply = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 20 ; down < 5\n\
             Supply level = 1.3.6.1.2.1.43.11.1.1.9 ; walk\n\
             Temperature = 1.3.6.1.4.1.6574.1.2.0 ; unit °C ; warn > 50 ; down > 60\n\
             Uptime = 1.3.6.1.2.1.1.3.0",
        )
        .unwrap();
        let printer = check(CheckSpec::snmp("10.0.0.9", oids), MetricLimits::default());
        let read = |values: &[(&str, f64, &str)], health: Health| CheckResult {
            check_id: CheckId("c".into()),
            status: health,
            message: "as the probe saw it".into(),
            latency_ms: Some(12),
            metrics: values
                .iter()
                .map(|(key, value, unit)| Metric::new(key, *value, unit))
                .collect(),
        };
        let levels = Levels::default();
        let judge =
            |values: &[(&str, f64, &str)]| levels.judge(&read(values, Health::Up), &printer);

        // Nothing past a limit; a number without limits is never judged.
        let fine = judge(&[
            ("Supply: Black", 85.0, "%"),
            ("Supply level: Black", 1.0, ""),
            ("Temperature", 50.0, "°C"),
            ("Uptime", 3.0, "s"),
        ]);
        assert_eq!((fine.status, fine.reason), (Status::Up, None));

        let low = judge(&[("Supply: Black", 85.0, "%"), ("Supply: Cyan", 12.34, "%")]);
        assert_eq!(low.status, Status::Warning);
        assert_eq!(
            low.reason.as_deref(),
            Some("Supply: Cyan 12.3 % is under the warning limit (20 %)")
        );

        // The worst one is named, the others counted.
        let several = judge(&[
            ("Supply: Cyan", 12.0, "%"),
            ("Temperature", 61.0, "°C"),
            ("Supply: Black", 4.0, "%"),
        ]);
        assert_eq!(several.status, Status::Down);
        assert_eq!(
            several.reason.as_deref(),
            Some("Temperature 61 °C is over the down limit (60 °C), and 2 more past their limits")
        );

        // What the probe says is down stays down, with its own message.
        let down = levels.judge(&read(&[("Supply: Cyan", 1.0, "%")], Health::Down), &printer);
        assert_eq!((down.status, down.reason), (Status::Down, None));
    }

    #[test]
    fn a_drives_warning_sign_is_a_warning_and_a_failing_drive_stays_down() {
        let result = |health: Health, failing: f64, warnings: f64, message: &str| CheckResult {
            check_id: CheckId("c".into()),
            status: health,
            message: message.into(),
            latency_ms: None,
            metrics: vec![
                Metric::new(metric::FAILING, failing, ""),
                Metric::new(metric::WARNINGS, warnings, ""),
                Metric::new(metric::DRIVES, 3.0, ""),
                Metric::new(metric::TEMPERATURE, 41.0, metric::CELSIUS),
            ],
        };
        let smart = check(CheckSpec::Smart { ignore: vec![] }, MetricLimits::default());
        let levels = Levels::default();
        let judged = |result: &CheckResult| {
            let verdict = levels.judge(result, &smart);
            (verdict.status, verdict.reason)
        };

        let healthy = result(
            Health::Up,
            0.0,
            0.0,
            "3 disks healthy, the hottest at 41 °C",
        );
        assert_eq!(judged(&healthy), (Status::Up, None));
        // The probe's words name the drive: they are the reason.
        let sectors = "ST4000DM004 (serial ZFN0): 8 bad sectors; 2 other disks healthy";
        assert_eq!(
            judged(&result(Health::Up, 0.0, 1.0, sectors)),
            (Status::Warning, Some(sectors.to_string()))
        );
        // A failing drive is the probe's Down, whatever else shows signs.
        let failing = result(
            Health::Down,
            1.0,
            1.0,
            "ST3000 (serial W1F2): the drive says it is failing (SMART)",
        );
        assert_eq!(judged(&failing), (Status::Down, None));
        // A probe that couldn't read the drives sends no numbers.
        let unread = CheckResult {
            metrics: vec![],
            ..result(
                Health::Down,
                0.0,
                0.0,
                "The disk service (udisks2) can't be read",
            )
        };
        assert_eq!(judged(&unread), (Status::Down, None));
    }

    #[test]
    fn events_are_trouble_or_must_be_there_as_the_check_says() {
        let result = |health: Health, events: Option<f64>, message: &str| CheckResult {
            check_id: CheckId("c".into()),
            status: health,
            message: message.into(),
            latency_ms: None,
            metrics: events
                .map(|events| Metric::new(metric::EVENTS, events, ""))
                .into_iter()
                .collect(),
        };
        let eventlog = |count, wanted, warning| {
            let mut spec = CheckSpec::eventlog("System");
            if let CheckSpec::Eventlog {
                count: c,
                wanted: w,
                warning: g,
                ..
            } = &mut spec
            {
                (*c, *w, *g) = (count, wanted, warning);
            }
            check(spec, MetricLimits::default())
        };
        let levels = Levels::default();
        let judged = |check: &CheckRecord, events: f64, message: &str| {
            let verdict = levels.judge(&result(Health::Up, Some(events), message), check);
            (verdict.status, verdict.reason)
        };
        let found = "2 events in the last 1 hour; the latest: disk 51 at 2026-10-12 14:02 UTC";
        let none = "No such event in the last 1 hour";

        // Events are trouble: one is enough, and the probe's words say which.
        let errors = eventlog(1, false, false);
        assert_eq!(judged(&errors, 0.0, none), (Status::Up, None));
        assert_eq!(
            judged(&errors, 2.0, found),
            (Status::Down, Some(found.to_string()))
        );
        // Only from so many on, and then a Warning if the check says so.
        let noisy = eventlog(3, false, true);
        assert_eq!(judged(&noisy, 2.0, found), (Status::Up, None));
        assert_eq!(
            judged(&noisy, 3.0, found),
            (Status::Warning, Some(found.to_string()))
        );

        // Events that must be there: a backup's "succeeded".
        let backup = eventlog(1, true, false);
        assert_eq!(judged(&backup, 1.0, found), (Status::Up, None));
        assert_eq!(
            judged(&backup, 0.0, none),
            (Status::Down, Some(none.to_string()))
        );
        let twice = eventlog(3, true, true);
        assert_eq!(
            judged(&twice, 2.0, found),
            (
                Status::Warning,
                Some(format!("Fewer than the 3 expected: {found}"))
            )
        );
        assert_eq!(judged(&twice, 3.0, found), (Status::Up, None));

        // A log the probe couldn't read is Down with the probe's words,
        // whatever the check wants; a result without a count isn't judged.
        for check in [&errors, &backup, &twice] {
            let down = levels.judge(&result(Health::Down, None, "No event log named X"), check);
            assert_eq!((down.status, down.reason), (Status::Down, None));
            let bare = levels.judge(&result(Health::Up, None, "?"), check);
            assert_eq!((bare.status, bare.reason), (Status::Up, None));
        }
    }

    #[test]
    fn waiting_updates_are_a_warning_by_the_checks_own_limits() {
        let result = |health: Health, metrics: &[(&str, f64)]| CheckResult {
            check_id: CheckId("c".into()),
            status: health,
            message: "what the probe said".into(),
            latency_ms: None,
            metrics: metrics
                .iter()
                .map(|(key, value)| Metric::new(key, *value, ""))
                .collect(),
        };
        let updates = |security_days, reboot_days, stale_days| {
            check(
                CheckSpec::Updates {
                    security_days,
                    reboot_days,
                    stale_days,
                },
                MetricLimits::default(),
            )
        };
        let levels = Levels::default();
        let judged = |check: &CheckRecord, metrics: &[(&str, f64)]| {
            let verdict = levels.judge(&result(Health::Up, metrics), check);
            (verdict.status, verdict.reason)
        };
        let usual = check(CheckSpec::updates(), MetricLimits::default());
        let fine = [
            (metric::UPDATES, 3.0),
            (metric::SECURITY, 0.0),
            (metric::REBOOT, 0.0),
            (metric::LOOKED_DAYS, 0.4),
        ];
        // Updates that aren't security updates only wait.
        assert_eq!(judged(&usual, &fine), (Status::Up, None));

        // The usual limits: at once for security updates and a restart,
        // two weeks without looking.
        let waiting = [
            (metric::UPDATES, 7.0),
            (metric::SECURITY, 2.0),
            (metric::SECURITY_DAYS, 0.0),
            (metric::REBOOT, 1.0),
            (metric::REBOOT_DAYS, 0.0),
            (metric::LOOKED_DAYS, 20.04),
        ];
        assert_eq!(
            judged(&usual, &waiting),
            (
                Status::Warning,
                Some(
                    "2 security updates wait; a restart is needed to finish updates; \
                     the system last looked for updates 20 days ago (limit 14)"
                        .into()
                )
            )
        );
        assert_eq!(
            judged(
                &usual,
                &[(metric::SECURITY, 1.0), (metric::SECURITY_DAYS, 0.0)]
            ),
            (Status::Warning, Some("1 security update waits".into()))
        );

        // Limits in days: only what has waited that long.
        let patient = updates(Some(7), Some(3), None);
        assert_eq!(judged(&patient, &waiting), (Status::Up, None));
        let overdue = [
            (metric::SECURITY, 2.0),
            (metric::SECURITY_DAYS, 9.5),
            (metric::REBOOT, 1.0),
            (metric::REBOOT_DAYS, 2.9),
            (metric::LOOKED_DAYS, 300.0),
        ];
        assert_eq!(
            judged(&patient, &overdue),
            (
                Status::Warning,
                Some("2 security updates have waited 9.5 days (limit 7)".into())
            )
        );
        assert_eq!(
            judged(
                &patient,
                &[(metric::REBOOT, 1.0), (metric::REBOOT_DAYS, 3.0)]
            ),
            (
                Status::Warning,
                Some("A restart has been needed for 3 days (limit 3)".into())
            )
        );
        assert_eq!(
            judged(
                &updates(Some(1), None, None),
                &[(metric::SECURITY, 1.0), (metric::SECURITY_DAYS, 1.0)]
            ),
            (
                Status::Warning,
                Some("1 security update has waited 1 day (limit 1)".into())
            )
        );
        // Every limit off: only reported.
        assert_eq!(
            judged(&updates(None, None, None), &overdue),
            (Status::Up, None)
        );
        // What the system doesn't say is not judged.
        assert_eq!(
            judged(&usual, &[(metric::UPDATES, 0.0)]),
            (Status::Up, None)
        );

        // A check the probe couldn't make is Down, with the probe's words.
        let down = levels.judge(&result(Health::Down, &[(metric::REBOOT, 1.0)]), &usual);
        assert_eq!((down.status, down.reason), (Status::Down, None));
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
    fn all_disks_are_judged_by_the_fullest_and_the_reason_names_it() {
        let levels = Levels::default();
        let disks = check(CheckSpec::Disks { ignore: vec![] }, MetricLimits::default());
        let mut result = disk(92.0, Health::Up);
        result.message = "D:\\ 92.0% (80.0 GB free), C:\\ 40.0% (60.0 GB free)".into();
        let verdict = levels.judge(&result, &disks);
        assert_eq!(verdict.status, Status::Warning);
        assert_eq!(
            verdict.reason.as_deref(),
            Some(
                "disk usage 92% is over the warning limit (90%, from the built-in default): \
                 D:\\ 92.0% (80.0 GB free), C:\\ 40.0% (60.0 GB free)"
            )
        );
        // One disk's check needs no more than the reason.
        let one = check(CheckSpec::Disk { path: None }, MetricLimits::default());
        assert!(!levels.judge(&result, &one).reason.unwrap().contains("D:"));
        result.metrics[0].value = 50.0;
        assert_eq!(levels.judge(&result, &disks).reason, None);
    }

    #[test]
    fn a_ups_on_battery_is_a_warning_and_a_low_battery_stays_down() {
        let levels = Levels::default();
        let nut = check(
            CheckSpec::Nut {
                host: "nas.lan".into(),
                port: None,
                ups: String::new(),
            },
            MetricLimits::default(),
        );
        let says = |health: Health, message: &str, warnings: f64| {
            let mut result = disk(0.0, health);
            result.message = message.into();
            result.metrics = vec![
                Metric::new(metric::CHARGE, 85.0, metric::PERCENT),
                Metric::new(metric::WARNINGS, warnings, ""),
            ];
            let verdict = levels.judge(&result, &nut);
            (verdict.status, verdict.reason)
        };
        assert_eq!(
            says(Health::Up, "Back-UPS: on mains power, battery 85 %", 0.0),
            (Status::Up, None)
        );
        let outage = "Back-UPS: on battery, battery 85 %, 31 min of runtime";
        assert_eq!(
            says(Health::Up, outage, 1.0),
            (Status::Warning, Some(outage.to_string()))
        );
        // The UPS's own alarm is the probe's Down.
        assert_eq!(
            says(Health::Down, "Back-UPS: on battery, battery low", 1.0).0,
            Status::Down
        );
    }

    #[test]
    fn a_clock_is_judged_by_how_far_it_is_off() {
        let levels = Levels::default();
        let ntp = check(
            CheckSpec::Ntp {
                host: "pool.ntp.org".into(),
                port: None,
            },
            MetricLimits::default(),
        );
        let off = |seconds: f64| {
            let mut result = disk(0.0, Health::Up);
            result.message = "This machine's clock is off (stratum 2)".into();
            result.metrics = vec![
                Metric::new(metric::OFFSET, seconds, metric::SECONDS),
                Metric::new(metric::STRATUM, 2.0, ""),
            ];
            result
        };
        assert_eq!(levels.judge(&off(0.012), &ntp).status, Status::Up);
        assert_eq!(levels.judge(&off(4.9), &ntp).status, Status::Up);
        let late = levels.judge(&off(12.34), &ntp);
        assert_eq!(late.status, Status::Warning);
        assert_eq!(
            late.reason.as_deref(),
            Some(
                "clock offset 12.3 s is over the warning limit (5 s, from the built-in default): \
                 This machine's clock is off (stratum 2)"
            )
        );
        assert_eq!(
            levels.judge(&off(60.0), &ntp).status,
            Status::Down,
            "at the limit"
        );
        assert_eq!(levels.judge(&off(7500.0), &ntp).status, Status::Down);

        // A check's own limits: a machine that must be within half a second.
        let strict = check(
            CheckSpec::Ntp {
                host: "dc01".into(),
                port: None,
            },
            MetricLimits {
                warning: Some(Setting::At(0.5)),
                down: Some(Setting::Off),
            },
        );
        assert_eq!(levels.judge(&off(0.6), &strict).status, Status::Warning);
        assert_eq!(levels.judge(&off(7500.0), &strict).status, Status::Warning);
    }

    #[test]
    fn a_mail_server_is_judged_by_its_latency_and_its_certificates_days() {
        use common::checks::{MailProtocol, MailSecurity};
        let levels = Levels::default();
        let mail = check(
            CheckSpec::Mail {
                protocol: MailProtocol::Smtp,
                host: "mail.example.com".into(),
                port: None,
                security: MailSecurity::Starttls,
                verify: true,
            },
            MetricLimits {
                warning: Some(Setting::At(500.0)),
                down: None,
            },
        );
        let answer = |latency_ms: u64, days_left: Option<f64>| {
            let mut result = disk(0.0, Health::Up);
            result.message = "SMTP ready (STARTTLS): mail.example.com ESMTP".into();
            result.latency_ms = Some(latency_ms);
            result.metrics = days_left
                .map(|days| Metric::new(metric::DAYS_LEFT, days, metric::DAYS))
                .into_iter()
                .collect();
            result
        };
        assert_eq!(
            levels.judge(&answer(120, Some(60.0)), &mail).status,
            Status::Up
        );
        // The check's own limit is its latency's.
        let slow = levels.judge(&answer(800, Some(60.0)), &mail);
        assert_eq!(slow.status, Status::Warning);
        assert!(slow.reason.unwrap().starts_with("latency 800 ms is over"));
        // Its certificate is judged by the certificate limits of the levels
        // above (21 and 7 days built in), as a `tls` check's is.
        let soon = levels.judge(&answer(120, Some(12.5)), &mail);
        assert_eq!(soon.status, Status::Warning);
        assert_eq!(
            soon.reason.as_deref(),
            Some(
                "certificate expiry in 12.5 days is under the warning limit (21 days, from the \
                 built-in default)"
            )
        );
        assert_eq!(
            levels.judge(&answer(120, Some(3.0)), &mail).status,
            Status::Down
        );
        // The worse of the two counts.
        assert_eq!(
            levels.judge(&answer(800, Some(3.0)), &mail).status,
            Status::Down
        );
        // Not encrypted: no certificate, nothing to judge there.
        assert_eq!(levels.judge(&answer(120, None), &mail).status, Status::Up);
        let mut down = answer(120, Some(60.0));
        down.status = Health::Down;
        assert_eq!(levels.judge(&down, &mail).status, Status::Down);
    }

    #[test]
    fn a_temperature_is_judged_by_its_limits_and_the_reason_names_the_sensor() {
        let levels = Levels::default();
        let all = check(
            CheckSpec::Temperature { sensors: vec![] },
            MetricLimits::default(),
        );
        let reading = |celsius: f64| {
            let mut result = disk(0.0, Health::Up);
            result.message =
                format!("coretemp Package id 0 is the hottest of 5 sensors, at {celsius:.0} °C");
            result.metrics = vec![
                Metric::new(metric::TEMPERATURE, celsius, metric::CELSIUS),
                Metric::new(metric::SENSORS, 5.0, ""),
            ];
            result
        };
        assert_eq!(levels.judge(&reading(73.0), &all).status, Status::Up);
        let hot = levels.judge(&reading(91.5), &all);
        assert_eq!(hot.status, Status::Warning);
        assert_eq!(
            hot.reason.as_deref(),
            Some(
                "temperature 91.5 °C is over the warning limit (90 °C, from the built-in \
                 default): coretemp Package id 0 is the hottest of 5 sensors, at 92 °C"
            )
        );
        // No built-in down limit: the hardware's own critical limit is the
        // probe's Down. A check's own limits apply.
        assert_eq!(levels.judge(&reading(120.0), &all).status, Status::Warning);
        let strict = check(
            CheckSpec::Temperature {
                sensors: vec!["nvme*".into()],
            },
            MetricLimits {
                warning: Some(Setting::At(55.0)),
                down: Some(Setting::At(70.0)),
            },
        );
        assert_eq!(levels.judge(&reading(54.9), &strict).status, Status::Up);
        assert_eq!(
            levels.judge(&reading(60.0), &strict).status,
            Status::Warning
        );
        assert_eq!(levels.judge(&reading(70.0), &strict).status, Status::Down);
        // The probe's Down (no sensor, a critical limit) stands.
        let mut none = reading(40.0);
        none.status = Health::Down;
        none.metrics.clear();
        assert_eq!(levels.judge(&none, &all).status, Status::Down);
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
