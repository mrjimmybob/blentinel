//! Check dependencies (M3-07, proposal 0005 §2.5, D-68): a check can depend
//! on another check of its probe, usually the site's gateway ping. While the
//! parent is Down, the checks behind it are **Unknown (unreachable)**: their
//! results say nothing about them, so they don't alert, don't open incidents
//! and don't count for uptime; the parent's alert names them.
//!
//! Pure functions: [`apply`] judges a report's results (after thresholds),
//! [`validate`] a new link.

use std::collections::{HashMap, HashSet};

use common::checks::{CheckId, CheckResult};

use crate::model::{Evaluation, Status};
use crate::storage::CheckRecord;

/// The check's ancestors, nearest first (stops at a cycle or a missing one).
fn ancestors<'a>(checks: &'a [CheckRecord], of: &CheckRecord) -> Vec<&'a CheckRecord> {
    let by_id: HashMap<&CheckId, &CheckRecord> = checks.iter().map(|c| (&c.id, c)).collect();
    let mut seen = HashSet::from([&of.id]);
    let mut chain = Vec::new();
    let mut next = of.depends_on.as_ref();
    while let Some(id) = next {
        let Some(parent) = by_id.get(id) else { break };
        if !seen.insert(&parent.id) {
            break;
        }
        chain.push(*parent);
        next = parent.depends_on.as_ref();
    }
    chain
}

/// Mark the results (in `evaluations`, in the order of `results`) whose
/// check has an ancestor Down in the same report as Unknown, behind the
/// farthest one that is: the root cause (a router behind which a switch,
/// and behind that a NAS, are unreachable). Judged on the statuses as they
/// came in, so the order of the results doesn't matter.
pub fn apply(results: &[CheckResult], checks: &[CheckRecord], evaluations: &mut [Evaluation]) {
    let down: HashSet<&CheckId> = results
        .iter()
        .zip(evaluations.iter())
        .filter(|(_, e)| e.status == Status::Down)
        .map(|(r, _)| &r.check_id)
        .collect();
    let mut unreachable = Vec::new();
    for (i, result) in results.iter().enumerate() {
        let Some(check) = checks.iter().find(|c| c.id == result.check_id) else {
            continue;
        };
        let root = ancestors(checks, check)
            .into_iter()
            .rfind(|a| down.contains(&a.id));
        if let Some(root) = root {
            unreachable.push((i, root));
        }
    }
    for (i, root) in unreachable {
        evaluations[i] = Evaluation {
            status: Status::Unknown,
            reason: Some(format!("unreachable: {} is down", root.name)),
            behind: Some(root.id.to_string()),
            ..evaluations[i].clone()
        };
    }
}

/// Can `check` (its ID; `None`: a new one) depend on `parent`, given its
/// probe's `checks`? `Err` with why not.
pub fn validate(
    checks: &[CheckRecord],
    check: Option<&CheckId>,
    parent: &str,
) -> Result<CheckId, String> {
    let Some(parent) = checks.iter().find(|c| c.id.as_str() == parent) else {
        return Err("a check depends on another check of the same probe".into());
    };
    let Some(check) = check else {
        return Ok(parent.id.clone());
    };
    if &parent.id == check {
        return Err("a check can't depend on itself".into());
    }
    let mut chain = ancestors(checks, parent).into_iter().map(|a| &a.id);
    if chain.any(|id| id == check) {
        return Err(format!(
            "{} already depends on this check (directly or not)",
            parent.name
        ));
    }
    Ok(parent.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use common::checks::{CheckSpec, Health};

    fn check(id: &str, parent: Option<&str>) -> CheckRecord {
        CheckRecord {
            id: CheckId(id.into()),
            probe_id: "p".into(),
            company_id: "acme".into(),
            name: id.to_uppercase(),
            spec: CheckSpec::ping("10.0.0.1"),
            enabled: true,
            alert_after: Default::default(),
            limits: Default::default(),
            depends_on: parent.map(|p| CheckId(p.into())),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: None,
        }
    }

    fn result(id: &str, up: bool) -> CheckResult {
        CheckResult {
            check_id: CheckId(id.into()),
            status: if up { Health::Up } else { Health::Down },
            message: String::new(),
            latency_ms: None,
            metrics: Vec::new(),
        }
    }

    /// The statuses after [`apply`], for results `(id, up)`.
    fn judge(checks: &[CheckRecord], results: &[(&str, bool)]) -> Vec<(Status, Option<String>)> {
        let results: Vec<CheckResult> = results.iter().map(|(id, up)| result(id, *up)).collect();
        let mut evaluations: Vec<Evaluation> = results
            .iter()
            .map(|r| Evaluation::of_health(r.status))
            .collect();
        apply(&results, checks, &mut evaluations);
        evaluations
            .into_iter()
            .map(|e| (e.status, e.behind))
            .collect()
    }

    #[test]
    fn behind_a_down_parent_a_check_is_unknown() {
        let checks = [
            check("gw", None),
            check("nas", Some("gw")),
            check("www", None),
        ];
        let judged = judge(&checks, &[("nas", false), ("gw", false), ("www", false)]);
        assert_eq!(
            judged,
            [
                (Status::Unknown, Some("gw".into())),
                (Status::Down, None),
                (Status::Down, None),
            ]
        );
        // The parent up: the child is what it says.
        let judged = judge(&checks, &[("gw", true), ("nas", false)]);
        assert_eq!(judged[1], (Status::Down, None));
    }

    #[test]
    fn the_farthest_down_ancestor_is_the_root_cause() {
        let checks = [
            check("router", None),
            check("switch", Some("router")),
            check("nas", Some("switch")),
        ];
        let judged = judge(
            &checks,
            &[("router", false), ("switch", false), ("nas", false)],
        );
        assert_eq!(judged[1].1.as_deref(), Some("router"));
        assert_eq!(judged[2].1.as_deref(), Some("router"));
        // Only the switch down: the NAS is behind it.
        let judged = judge(
            &checks,
            &[("router", true), ("switch", false), ("nas", true)],
        );
        assert_eq!(judged[2], (Status::Unknown, Some("switch".into())));
    }

    #[test]
    fn a_parent_without_a_result_changes_nothing() {
        let checks = [check("gw", None), check("nas", Some("gw"))];
        assert_eq!(judge(&checks, &[("nas", false)]), [(Status::Down, None)]);
    }

    #[test]
    fn links_stay_on_the_probe_and_never_loop() {
        let checks = [
            check("a", None),
            check("b", Some("a")),
            check("c", Some("b")),
        ];
        assert_eq!(validate(&checks, None, "a"), Ok(CheckId("a".into())));
        assert!(validate(&checks, None, "elsewhere").is_err());
        let id = |s: &str| CheckId(s.into());
        assert!(
            validate(&checks, Some(&id("a")), "a")
                .unwrap_err()
                .contains("itself")
        );
        assert!(
            validate(&checks, Some(&id("a")), "c")
                .unwrap_err()
                .contains("C already depends")
        );
        assert_eq!(validate(&checks, Some(&id("c")), "a"), Ok(id("a")));
    }
}
