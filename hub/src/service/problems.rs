//! What is wrong now (M6-03, proposal 0007 §3.3, D-114): every check that
//! is Down, Warning or Unknown and every probe that stopped reporting, in
//! the user's companies, grouped by company › site, with the open incident
//! and who acknowledged it. Searched and filtered here, on the hub.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Utc};

use crate::model::{
    CompanyScope, PROBLEM_STATUSES, Problem, ProblemCompany, ProblemIncident, ProblemSite,
    ProblemsView, Status,
};
use crate::storage::{Storage, StoredIncident, StoredProblem};

/// Open incidents read at most (one per check or probe can be open).
const OPEN_INCIDENTS: u32 = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum ProblemsError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// What to show: words to find, statuses, a check kind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProblemsFilter {
    /// Lowercase; each must be in the company, site, probe, check or target.
    pub words: Vec<String>,
    /// Empty: every status.
    pub statuses: BTreeSet<u8>,
    /// `None`: every kind. A kind leaves out probes that stopped reporting.
    pub kind: Option<String>,
}

impl ProblemsFilter {
    /// From `?q=`, `?status=down,warning` and `?kind=ping`.
    pub fn parse(
        q: Option<&str>,
        status: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Self, ProblemsError> {
        let words = q
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut statuses = BTreeSet::new();
        for part in status.unwrap_or_default().split(',').map(str::trim) {
            if part.is_empty() {
                continue;
            }
            let found = PROBLEM_STATUSES
                .into_iter()
                .find(|s| s.as_str().eq_ignore_ascii_case(part))
                .ok_or_else(|| {
                    ProblemsError::Invalid(format!(
                        "status is down, warning or unknown, not {part:?}"
                    ))
                })?;
            statuses.insert(found.rank());
        }
        let kind = kind
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_lowercase);
        Ok(Self {
            words,
            statuses,
            kind,
        })
    }

    /// Does the search find `problem`?
    fn finds(&self, p: &StoredProblem) -> bool {
        let check = p.check.as_ref();
        let target = check.map(|c| c.spec.target());
        let haystack = [
            Some(p.company_name.as_str()),
            Some(p.company_id.as_str()),
            Some(p.site_name.as_str()),
            Some(p.probe_name.as_str()),
            check.map(|c| c.name.as_str()),
            target.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
        self.words
            .iter()
            .all(|word| haystack.iter().any(|field| field.contains(word.as_str())))
    }

    fn kind_matches(&self, p: &StoredProblem) -> bool {
        match (&self.kind, &p.check) {
            (None, _) => true,
            (Some(kind), Some(check)) => check.spec.kind() == kind,
            (Some(_), None) => false,
        }
    }

    fn status_matches(&self, status: Status) -> bool {
        self.statuses.is_empty() || self.statuses.contains(&status.rank())
    }
}

/// A problem's status: a probe that stopped reporting is Unknown.
fn status_of(p: &StoredProblem) -> Status {
    p.check.as_ref().map_or(Status::Unknown, |c| c.status)
}

fn minute(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

/// What is wrong now in `scope`, as `filter` asks.
pub async fn problems(
    store: &dyn Storage,
    scope: &CompanyScope,
    filter: &ProblemsFilter,
) -> Result<ProblemsView, ProblemsError> {
    let all = store.current_problems(scope).await?;
    let incidents = store.incidents(scope, Some(true), OPEN_INCIDENTS).await?;
    Ok(view(all, &incidents, filter))
}

/// Filter, attach incidents, group and sort (the part with no storage).
pub fn view(
    all: Vec<StoredProblem>,
    incidents: &[StoredIncident],
    filter: &ProblemsFilter,
) -> ProblemsView {
    let total = u32::try_from(all.len()).unwrap_or(u32::MAX);
    let kinds: BTreeSet<String> = all
        .iter()
        .filter_map(|p| p.check.as_ref().map(|c| c.spec.kind().to_string()))
        .collect();
    let open: HashMap<(&str, Option<&str>), &StoredIncident> = incidents
        .iter()
        .map(|i| ((i.probe_id.as_str(), i.check_id.as_deref()), i))
        .collect();

    let found: Vec<StoredProblem> = all
        .into_iter()
        .filter(|p| filter.finds(p) && filter.kind_matches(p))
        .collect();
    let count = |status: Status| {
        u32::try_from(found.iter().filter(|p| status_of(p) == status).count()).unwrap_or(u32::MAX)
    };
    let (down, warning, unknown) = (
        count(Status::Down),
        count(Status::Warning),
        count(Status::Unknown),
    );

    // company › site › problems, by ID while grouping.
    type Sites = BTreeMap<String, (String, Vec<Problem>)>;
    let mut companies: BTreeMap<String, (String, Sites)> = BTreeMap::new();
    for p in found
        .into_iter()
        .filter(|p| filter.status_matches(status_of(p)))
    {
        let status = status_of(&p);
        let incident = open
            .get(&(p.probe_id.as_str(), p.check.as_ref().map(|c| c.id.as_str())))
            .map(|i| ProblemIncident {
                id: i.id,
                opened_at: minute(i.opened_at),
                acknowledged_by: i.acknowledged_by.clone(),
                acknowledged_at: i.acknowledged_at.map(minute),
            });
        let problem = Problem {
            probe_id: p.probe_id.clone(),
            probe_name: p.probe_name.clone(),
            check_id: p.check.as_ref().map(|c| c.id.clone()),
            check_name: p.check.as_ref().map(|c| c.name.clone()),
            kind: p.check.as_ref().map(|c| c.spec.kind().to_string()),
            target: p.check.as_ref().map(|c| c.spec.target()),
            status,
            reason: p.check.as_ref().and_then(|c| c.reason.clone()),
            message: p.check.as_ref().and_then(|c| c.message.clone()),
            in_maintenance: p.check.as_ref().is_some_and(|c| c.in_maintenance),
            last_seen_at: p.last_seen_at.map(minute),
            incident,
        };
        companies
            .entry(p.company_id.clone())
            .or_insert_with(|| (p.company_name.clone(), BTreeMap::new()))
            .1
            .entry(p.site_id.clone())
            .or_insert_with(|| (p.site_name.clone(), Vec::new()))
            .1
            .push(problem);
    }

    let worst = |problems: &[Problem]| {
        problems
            .iter()
            .map(|p| p.status)
            .min_by_key(|s| s.rank())
            .unwrap_or(Status::Unknown)
    };
    let mut companies: Vec<ProblemCompany> = companies
        .into_iter()
        .map(|(company_id, (name, sites))| {
            let mut sites: Vec<ProblemSite> = sites
                .into_iter()
                .map(|(site_id, (name, mut problems))| {
                    problems.sort_by_key(|p| {
                        (
                            p.status.rank(),
                            p.check_id.is_some(),
                            p.probe_name.to_lowercase(),
                            p.check_name.clone().unwrap_or_default().to_lowercase(),
                        )
                    });
                    ProblemSite {
                        site_id,
                        name,
                        problems,
                    }
                })
                .collect();
            sites.sort_by_key(|s| (worst(&s.problems).rank(), s.name.to_lowercase()));
            let all: Vec<Problem> = sites.iter().flat_map(|s| s.problems.clone()).collect();
            ProblemCompany {
                company_id,
                name,
                worst: worst(&all),
                sites,
            }
        })
        .collect();
    companies.sort_by_key(|c| (c.worst.rank(), c.name.to_lowercase()));

    ProblemsView {
        total,
        down,
        warning,
        unknown,
        kinds: kinds.into_iter().collect(),
        companies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ProblemCheck;
    use common::checks::CheckSpec;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// A problem at `company` › `site` › `probe`: a check (name, spec,
    /// status), or `None` for the probe having stopped reporting.
    fn problem(
        company: &str,
        site: &str,
        probe: &str,
        check: Option<(&str, CheckSpec, Status)>,
    ) -> StoredProblem {
        StoredProblem {
            company_id: company.to_lowercase(),
            company_name: company.into(),
            site_id: site.to_lowercase(),
            site_name: site.into(),
            probe_id: format!("{probe}-id"),
            probe_name: probe.into(),
            last_seen_at: Some(t("2026-03-10T11:00:00Z")),
            check: check.map(|(name, spec, status)| ProblemCheck {
                id: format!("{probe}.{name}"),
                name: name.into(),
                spec,
                status,
                reason: None,
                message: Some("Request timed out".into()),
                in_maintenance: false,
            }),
        }
    }

    fn incident(probe: &str, check: Option<&str>, acknowledged_by: Option<&str>) -> StoredIncident {
        StoredIncident {
            id: 7,
            company_id: String::new(),
            company_name: String::new(),
            site_id: String::new(),
            site_name: String::new(),
            probe_id: format!("{probe}-id"),
            probe_name: probe.into(),
            check_id: check.map(|c| format!("{probe}.{c}")),
            check_name: check.map(str::to_string),
            severity: Status::Down,
            status: Status::Down,
            opened_at: t("2026-03-10T10:00:00Z"),
            closed_at: None,
            acknowledged_by: acknowledged_by.map(str::to_string),
            acknowledged_at: acknowledged_by.map(|_| t("2026-03-10T10:05:00Z")),
            announced: true,
            reminders: 0,
            last_reminded_at: None,
            escalated_at: None,
        }
    }

    fn all() -> Vec<StoredProblem> {
        vec![
            problem(
                "Zeta",
                "Main",
                "z1",
                Some(("NAS", CheckSpec::ping("10.0.0.20"), Status::Warning)),
            ),
            problem(
                "Acme",
                "HQ",
                "hq-1",
                Some((
                    "Web",
                    CheckSpec::http("https://shop.example"),
                    Status::Unknown,
                )),
            ),
            problem(
                "Acme",
                "HQ",
                "hq-1",
                Some(("Router", CheckSpec::ping("10.0.0.1"), Status::Down)),
            ),
            problem("Acme", "Branch", "br-1", None),
        ]
    }

    /// `(company, site, probe, check)` in the order shown.
    fn shown(view: &ProblemsView) -> Vec<(String, String, String, Option<String>)> {
        view.companies
            .iter()
            .flat_map(|c| {
                c.sites.iter().flat_map(move |s| {
                    s.problems.iter().map(move |p| {
                        (
                            c.name.clone(),
                            s.name.clone(),
                            p.probe_name.clone(),
                            p.check_name.clone(),
                        )
                    })
                })
            })
            .collect()
    }

    fn row(
        company: &str,
        site: &str,
        probe: &str,
        check: Option<&str>,
    ) -> (String, String, String, Option<String>) {
        (
            company.into(),
            site.into(),
            probe.into(),
            check.map(str::to_string),
        )
    }

    #[test]
    fn problems_are_grouped_by_company_and_site_worst_first() {
        let view = view(all(), &[], &ProblemsFilter::default());
        assert_eq!(view.total, 4);
        assert_eq!((view.down, view.warning, view.unknown), (1, 1, 2));
        assert_eq!(view.kinds, ["http", "ping"]);
        assert_eq!(
            shown(&view),
            [
                // Acme has a Down: first. Its HQ site has it: before Branch.
                row("Acme", "HQ", "hq-1", Some("Router")),
                row("Acme", "HQ", "hq-1", Some("Web")),
                row("Acme", "Branch", "br-1", None),
                row("Zeta", "Main", "z1", Some("NAS")),
            ]
        );
        assert_eq!(view.companies[0].worst, Status::Down);
        assert_eq!(view.companies[1].worst, Status::Warning);
        let stopped = &view.companies[0].sites[1].problems[0];
        assert_eq!(
            (stopped.status, stopped.kind.as_deref()),
            (Status::Unknown, None)
        );
        assert_eq!(stopped.last_seen_at.as_deref(), Some("2026-03-10 11:00"));
        let router = &view.companies[0].sites[0].problems[0];
        assert_eq!(router.target.as_deref(), Some("10.0.0.1"));
        assert_eq!(router.kind.as_deref(), Some("ping"));
    }

    #[test]
    fn each_problem_shows_its_open_incident_and_who_acknowledged_it() {
        let incidents = [
            incident("hq-1", Some("Router"), Some("alice")),
            incident("br-1", None, None),
        ];
        let view = view(all(), &incidents, &ProblemsFilter::default());
        let acme = &view.companies[0];
        let router = acme.sites[0].problems[0].incident.as_ref().unwrap();
        assert_eq!(router.id, 7);
        assert_eq!(router.opened_at, "2026-03-10 10:00");
        assert_eq!(router.acknowledged_by.as_deref(), Some("alice"));
        assert_eq!(router.acknowledged_at.as_deref(), Some("2026-03-10 10:05"));
        assert!(
            acme.sites[0].problems[1].incident.is_none(),
            "Web: no incident"
        );
        let probe = acme.sites[1].problems[0].incident.as_ref().unwrap();
        assert_eq!(probe.acknowledged_by, None, "the probe's own incident");
    }

    #[test]
    fn search_finds_every_word_in_any_name_or_target() {
        let filter = |q: &str| ProblemsFilter::parse(Some(q), None, None).unwrap();
        let find = |q: &str| shown(&view(all(), &[], &filter(q)));
        assert_eq!(find("router"), [row("Acme", "HQ", "hq-1", Some("Router"))]);
        assert_eq!(
            find("10.0.0.20"),
            [row("Zeta", "Main", "z1", Some("NAS"))],
            "target"
        );
        assert_eq!(
            find("shop.example"),
            [row("Acme", "HQ", "hq-1", Some("Web"))]
        );
        assert_eq!(
            find("BRANCH"),
            [row("Acme", "Branch", "br-1", None)],
            "site, any case"
        );
        assert_eq!(find("acme hq").len(), 2, "every word, in different fields");
        assert_eq!(
            find("zeta"),
            [row("Zeta", "Main", "z1", Some("NAS"))],
            "company"
        );
        assert!(find("acme nas").is_empty(), "not all words in one problem");
        assert_eq!(find("").len(), 4);
        // The counts follow the search; the total and kinds don't.
        let found = view(all(), &[], &filter("acme"));
        assert_eq!(
            (found.total, found.down, found.warning, found.unknown),
            (4, 1, 0, 2)
        );
        assert_eq!(found.kinds, ["http", "ping"]);
    }

    #[test]
    fn status_and_kind_filters_narrow_the_list() {
        let filter = |status: Option<&str>, kind: Option<&str>| {
            ProblemsFilter::parse(None, status, kind).unwrap()
        };
        let down = view(all(), &[], &filter(Some("down"), None));
        assert_eq!(shown(&down), [row("Acme", "HQ", "hq-1", Some("Router"))]);
        assert_eq!(
            (down.down, down.warning, down.unknown),
            (1, 1, 2),
            "counts ignore the status filter"
        );
        let two = view(all(), &[], &filter(Some("Warning, unknown"), None));
        assert_eq!(shown(&two).len(), 3);
        let ping = view(all(), &[], &filter(None, Some("ping")));
        assert_eq!(
            shown(&ping),
            [
                row("Acme", "HQ", "hq-1", Some("Router")),
                row("Zeta", "Main", "z1", Some("NAS")),
            ],
            "a kind leaves out stopped probes"
        );
        assert_eq!((ping.down, ping.warning, ping.unknown), (1, 1, 0));
        let both = view(all(), &[], &filter(Some("warning"), Some("ping")));
        assert_eq!(shown(&both), [row("Zeta", "Main", "z1", Some("NAS"))]);
        assert!(
            view(all(), &[], &filter(None, Some("dns")))
                .companies
                .is_empty()
        );
    }

    #[test]
    fn a_status_must_be_one_the_list_shows() {
        assert!(ProblemsFilter::parse(None, Some("up"), None).is_err());
        assert!(ProblemsFilter::parse(None, Some("down,broken"), None).is_err());
        assert_eq!(
            ProblemsFilter::parse(Some("  "), Some(""), Some("")).unwrap(),
            ProblemsFilter::default()
        );
    }
}
