//! The managed probe rollout (M4-01, proposal 0012 §7, D-74): which release
//! each probe is offered, and when.
//!
//! - **The rollout.** The admin approves a release. If some probes are
//!   **canaries**, only they are offered it first; once every canary runs it
//!   (a probe reports a new version only after the version delivered a
//!   report, D-55) and the soak time has passed, the hub offers it to every
//!   probe ([`advance`]). A canary that refuses it or rolls back **pauses**
//!   the rollout. An admin can pause, resume, or send it to every probe now.
//! - **Per company:** follow the rollout (the default), take no updates, or
//!   be pinned to one release whatever is rolled out; and optionally an
//!   **update window** in each probe's site's local time. The hub only offers
//!   inside the window, so the probe needs no clock rules of its own.
//! - **Going back:** a release older than what a probe runs is offered only
//!   if its signed rollback names that version; the probe checks it again.
//!
//! The decisions ([`plan`], [`check_release`], [`canary_step`]) are pure;
//! the rest reads storage and the release folder around them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration, Timelike, Utc};
use common::ack::UpdateOffer;
use common::models::UpdateFailure;
use common::release::Version;

use crate::model::{
    CompanyScope, CompanyUpdatePolicy, CompanyUpdates, DEFAULT_SOAK_MINUTES, MAX_SOAK_MINUTES,
    ProbeInfo, RolloutInfo, RolloutPause, RolloutStage, UpdateMode, UpdateStatus,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::{maintenance, releases};
use crate::storage::{ProbeRecord, Rollout, RolloutChange, Storage};

/// Why a rollout change was refused.
#[derive(Debug, thiserror::Error)]
pub enum RolloutError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    /// The rollout isn't in a state the change applies to.
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// What [`plan`] needs to know about one probe.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub rollout: Option<&'a Rollout>,
    pub policy: &'a CompanyUpdatePolicy,
    pub canary: bool,
    /// The version it reports (`None`: too old to say, or to update itself).
    pub runs: Option<&'a str>,
    /// The last update it refused or rolled back.
    pub failed: Option<&'a UpdateFailure>,
    /// Local time (hour, minute) at its site, for its company's window.
    pub local: (u32, u32),
}

/// The release to offer a probe (`Ok`), or why none (`Err`; including
/// [`UpdateStatus::UpToDate`]). That the release is on the hub, and may go
/// to that version, is [`check_release`].
pub fn plan(facts: &Facts<'_>) -> Result<String, UpdateStatus> {
    let runs = facts.runs.ok_or(UpdateStatus::NoVersion)?;
    let (version, rollout) = match facts.policy.mode {
        UpdateMode::Off => return Err(UpdateStatus::CompanyOff),
        UpdateMode::Pin => (facts.policy.version.clone().unwrap_or_default(), None),
        UpdateMode::Follow => {
            let rollout = facts.rollout.ok_or(UpdateStatus::NoneApproved)?;
            (rollout.version.clone(), Some(rollout))
        }
    };
    if runs == version {
        return Err(UpdateStatus::UpToDate);
    }
    // It would refuse it again (D-55).
    if facts.failed.is_some_and(|f| f.version == version) {
        return Err(UpdateStatus::Refused { version });
    }
    if let Some(rollout) = rollout {
        if rollout.paused.is_some() {
            return Err(UpdateStatus::Paused { version });
        }
        if rollout.stage == RolloutStage::Canary && !facts.canary {
            return Err(UpdateStatus::AwaitingCanaries { version });
        }
    }
    if let Some(window) = &facts.policy.window
        && !window.contains(facts.local.0, facts.local.1)
    {
        return Err(UpdateStatus::OutsideWindow {
            version,
            window: window.clone(),
        });
    }
    Ok(version)
}

/// Can release `version` (`stored`, if it is on the hub) go to a probe that
/// runs `runs`? An older one only with a signed rollback naming `runs`.
pub fn check_release(
    version: &str,
    runs: &str,
    stored: Option<&releases::Stored>,
) -> Result<(), UpdateStatus> {
    let stored = stored.ok_or_else(|| UpdateStatus::Missing {
        version: version.into(),
    })?;
    let older = matches!(
        (Version::parse(version), Version::parse(runs)),
        (Some(to), Some(from)) if to < from
    );
    let allowed = stored.rollback.as_ref().is_some_and(|r| r.allows(runs));
    if older && !allowed {
        return Err(UpdateStatus::NoRollback {
            version: version.into(),
        });
    }
    Ok(())
}

/// Local (hour, minute) at `now` in time zone `name` (UTC if unknown).
pub fn local_time(name: Option<&str>, now: DateTime<Utc>) -> (u32, u32) {
    match name.and_then(|name| maintenance::zone(name).ok()) {
        Some(tz) => {
            let local = now.with_timezone(&tz);
            (local.hour(), local.minute())
        }
        None => (now.hour(), now.minute()),
    }
}

/// The offer for a probe that just reported, if it should get one.
pub async fn offer_for(
    storage: &dyn Storage,
    releases_dir: &Path,
    release_keys: &Arc<Vec<[u8; 32]>>,
    record: &ProbeRecord,
    runs: Option<&str>,
    failed: Option<&UpdateFailure>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<UpdateOffer>> {
    let Some(runs) = runs else {
        return Ok(None);
    };
    let rollout = storage.rollout().await?;
    let policy = storage.company_update_policy(&record.company_id).await?;
    let local = match &policy.window {
        Some(_) => {
            let tz = storage
                .site_timezone(&record.company_id, &record.site_id)
                .await?;
            local_time(tz.as_deref(), now)
        }
        None => (0, 0),
    };
    let facts = Facts {
        rollout: rollout.as_ref(),
        policy: &policy,
        canary: record.canary,
        runs: Some(runs),
        failed,
        local,
    };
    let Ok(version) = plan(&facts) else {
        return Ok(None);
    };
    let (root, keys, wanted) = (
        releases_dir.to_path_buf(),
        Arc::clone(release_keys),
        version.clone(),
    );
    let found =
        tokio::task::spawn_blocking(move || releases::find_with_offer(&root, &wanted, &keys))
            .await?;
    match check_release(&version, runs, found.as_ref().map(|(stored, _)| stored)) {
        Ok(()) => Ok(found.map(|(_, offer)| offer)),
        Err(UpdateStatus::Missing { .. }) => {
            tracing::error!(
                %version,
                "The probe release to offer is missing or no longer checks out; nothing is offered"
            );
            Ok(None)
        }
        Err(_) => Ok(None),
    }
}

/// The canaries that take part in a rollout: active probes marked canary
/// whose company follows it.
fn canaries<'a>(
    probes: &'a [ProbeInfo],
    policies: &HashMap<String, CompanyUpdatePolicy>,
) -> Vec<&'a ProbeInfo> {
    probes
        .iter()
        .filter(|p| p.canary && p.state == crate::storage::PROBE_ACTIVE)
        .filter(|p| {
            policies
                .get(&p.company_id)
                .is_none_or(|policy| policy.mode == UpdateMode::Follow)
        })
        .collect()
}

/// What the canary stage of a rollout should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Wait,
    /// Every canary runs it, from now.
    MarkReady,
    /// Not every canary runs it any more.
    MarkNotReady,
    Pause(String),
    Promote,
}

/// The next step of `rollout` (in its canary stage, not paused) given its
/// `canaries` at `now`.
pub fn canary_step(rollout: &Rollout, canaries: &[&ProbeInfo], now: DateTime<Utc>) -> Step {
    if canaries.is_empty() {
        return Step::Promote;
    }
    if let Some((probe, failed)) = canaries.iter().find_map(|p| {
        p.update_failed
            .as_ref()
            .filter(|f| f.version == rollout.version)
            .map(|f| (p, f))
    }) {
        return Step::Pause(format!(
            "canary {} didn't install {}: {}",
            probe.name, rollout.version, failed.error
        ));
    }
    let all_run_it = canaries
        .iter()
        .all(|p| p.probe_version.as_deref() == Some(rollout.version.as_str()));
    match (all_run_it, rollout.canaries_ready_at) {
        (true, None) => Step::MarkReady,
        (true, Some(since)) if now >= since + soak(rollout) => Step::Promote,
        (false, Some(_)) => Step::MarkNotReady,
        _ => Step::Wait,
    }
}

fn soak(rollout: &Rollout) -> Duration {
    Duration::minutes(i64::from(rollout.soak_minutes))
}

/// Move the rollout on: promote it, or pause it, as its canaries say. Run
/// by the hub's background sweep.
pub async fn advance(storage: &dyn Storage, now: DateTime<Utc>) -> anyhow::Result<()> {
    let Some(rollout) = storage.rollout().await? else {
        return Ok(());
    };
    if rollout.stage == RolloutStage::All || rollout.paused.is_some() {
        return Ok(());
    }
    let probes = storage.probe_records(&CompanyScope::All).await?;
    let policies: HashMap<_, _> = storage
        .company_update_policies()
        .await?
        .into_iter()
        .collect();
    let canaries = canaries(&probes, &policies);
    let version = rollout.version.as_str();
    match canary_step(&rollout, &canaries, now) {
        Step::Wait => {}
        Step::MarkReady => {
            storage
                .change_rollout(version, &RolloutChange::CanariesReady(Some(now)), now)
                .await?;
        }
        Step::MarkNotReady => {
            storage
                .change_rollout(version, &RolloutChange::CanariesReady(None), now)
                .await?;
        }
        Step::Pause(reason) => {
            let change = RolloutChange::Pause {
                by: None,
                reason: &reason,
            };
            if storage.change_rollout(version, &change, now).await? {
                tracing::warn!(%version, "Probe rollout paused: {reason}");
                let event = Event::new(action::ROLLOUT_PAUSE)
                    .target("release", version)
                    .details(serde_json::json!({ "reason": reason }));
                audit::record(storage, &Actor::hub(), event, now).await;
            }
        }
        Step::Promote => {
            if storage
                .change_rollout(version, &RolloutChange::Promote { by: None }, now)
                .await?
            {
                tracing::info!(%version, "The canaries run it: rolling out to every probe");
                let event = Event::new(action::ROLLOUT_PROMOTE).target("release", version);
                audit::record(storage, &Actor::hub(), event, now).await;
            }
        }
    }
    Ok(())
}

/// Does release `version` exist (and check out) on the hub?
async fn release_exists(root: PathBuf, keys: Arc<Vec<[u8; 32]>>, version: String) -> bool {
    tokio::task::spawn_blocking(move || releases::find(&root, &version, &keys).is_some())
        .await
        .unwrap_or(false)
}

/// Start rolling out `version` (canaries first, if there are any), or stop
/// (`None`). The rollout it started.
pub async fn approve(
    storage: &dyn Storage,
    (root, keys): (PathBuf, Arc<Vec<[u8; 32]>>),
    version: Option<String>,
    soak_minutes: Option<u32>,
    by: &str,
    now: DateTime<Utc>,
) -> Result<Option<Rollout>, RolloutError> {
    let Some(version) = version else {
        storage.set_rollout(None).await?;
        return Ok(None);
    };
    let soak_minutes = soak_minutes.unwrap_or(DEFAULT_SOAK_MINUTES);
    if soak_minutes > MAX_SOAK_MINUTES {
        return Err(RolloutError::Invalid(format!(
            "wait at most {MAX_SOAK_MINUTES} minutes (a week) after the canaries"
        )));
    }
    if !release_exists(root, keys, version.clone()).await {
        return Err(RolloutError::NotFound(format!(
            "no release {version} on this hub"
        )));
    }
    let probes = storage.probe_records(&CompanyScope::All).await?;
    let policies: HashMap<_, _> = storage
        .company_update_policies()
        .await?
        .into_iter()
        .collect();
    let stage = if canaries(&probes, &policies).is_empty() {
        RolloutStage::All
    } else {
        RolloutStage::Canary
    };
    let rollout = Rollout {
        version,
        approved_by: Some(by.to_string()),
        approved_at: now,
        stage,
        soak_minutes,
        canaries_ready_at: None,
        promoted_at: None,
        promoted_by: None,
        paused: None,
    };
    storage.set_rollout(Some(&rollout)).await?;
    Ok(Some(rollout))
}

/// What an admin does to the rollout under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminChange {
    Pause,
    Resume,
    /// Send it to every probe now.
    Promote,
}

/// Apply `change` by `by` to the rollout under way; the version it applied to.
pub async fn change(
    storage: &dyn Storage,
    change: AdminChange,
    by: &str,
    now: DateTime<Utc>,
) -> Result<String, RolloutError> {
    let rollout = storage
        .rollout()
        .await?
        .ok_or_else(|| RolloutError::NotFound("no release is being rolled out".into()))?;
    let reason = format!("paused by {by}");
    let (stored, refusal) = match change {
        AdminChange::Pause => (
            RolloutChange::Pause {
                by: Some(by),
                reason: &reason,
            },
            "the rollout is already paused",
        ),
        AdminChange::Resume => (RolloutChange::Resume, "the rollout isn't paused"),
        AdminChange::Promote => (
            RolloutChange::Promote { by: Some(by) },
            "every probe is already offered it",
        ),
    };
    if !storage
        .change_rollout(&rollout.version, &stored, now)
        .await?
    {
        return Err(RolloutError::Conflict(refusal.into()));
    }
    Ok(rollout.version)
}

/// Make a probe a canary, or not.
pub async fn set_canary(
    storage: &dyn Storage,
    probe_id: &str,
    canary: bool,
) -> Result<(), RolloutError> {
    if storage.set_probe_canary(probe_id, canary).await? {
        Ok(())
    } else {
        Err(RolloutError::NotFound("no such probe".into()))
    }
}

/// Set a company's update policy; a pinned release must be on the hub.
pub async fn set_policy(
    storage: &dyn Storage,
    (root, keys): (PathBuf, Arc<Vec<[u8; 32]>>),
    company_id: &str,
    mut policy: CompanyUpdatePolicy,
    by: &str,
    now: DateTime<Utc>,
) -> Result<(), RolloutError> {
    policy.validate().map_err(RolloutError::Invalid)?;
    if storage.company(company_id).await?.is_none() {
        return Err(RolloutError::NotFound("no such company".into()));
    }
    if let Some(version) = &mut policy.version {
        *version = version.trim().to_string();
        if !release_exists(root, keys, version.clone()).await {
            return Err(RolloutError::NotFound(format!(
                "no release {version} on this hub"
            )));
        }
    }
    if let Some(window) = &mut policy.window {
        window.start = window.start.trim().to_string();
        window.end = window.end.trim().to_string();
    }
    storage
        .set_company_update_policy(company_id, &policy, Some(by), now)
        .await?;
    Ok(())
}

/// The versions the hub must keep: the one rolled out and every pinned one.
pub async fn versions_in_use(storage: &dyn Storage) -> anyhow::Result<Vec<String>> {
    let mut versions: Vec<String> = storage
        .company_update_policies()
        .await?
        .into_iter()
        .filter_map(|(_, policy)| policy.version)
        .collect();
    versions.extend(storage.rollout().await?.map(|r| r.version));
    Ok(versions)
}

fn fmt(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// What the admin page shows of the rollout, the probes (each with its
/// [`UpdateStatus`]) and the companies' policies, given the `releases` on
/// the hub.
pub async fn overview(
    storage: &dyn Storage,
    releases: &[releases::Stored],
    now: DateTime<Utc>,
) -> anyhow::Result<(Option<RolloutInfo>, Vec<ProbeInfo>, Vec<CompanyUpdates>)> {
    let rollout = storage.rollout().await?;
    let mut probes = storage.probe_records(&CompanyScope::All).await?;
    let policies: HashMap<_, _> = storage
        .company_update_policies()
        .await?
        .into_iter()
        .collect();
    let zones: HashMap<(String, String), String> = storage
        .site_timezones()
        .await?
        .into_iter()
        .map(|(company, site, tz)| ((company, site), tz))
        .collect();
    let by_version: HashMap<&str, &releases::Stored> = releases
        .iter()
        .map(|s| (s.manifest.version.as_str(), s))
        .collect();

    let info = rollout.as_ref().map(|r| {
        let canaries = canaries(&probes, &policies);
        let ready = canaries
            .iter()
            .filter(|p| p.probe_version.as_deref() == Some(r.version.as_str()))
            .count();
        RolloutInfo {
            version: r.version.clone(),
            approved_by: r.approved_by.clone(),
            approved_at: fmt(r.approved_at),
            stage: r.stage,
            soak_minutes: r.soak_minutes,
            canaries: u32::try_from(canaries.len()).unwrap_or(u32::MAX),
            canaries_ready: u32::try_from(ready).unwrap_or(u32::MAX),
            promote_at: (r.stage == RolloutStage::Canary && r.paused.is_none())
                .then_some(r.canaries_ready_at)
                .flatten()
                .map(|since| fmt(since + soak(r))),
            promoted_at: r.promoted_at.map(fmt),
            promoted_by: r.promoted_by.clone(),
            paused: r.paused.as_ref().map(|p| RolloutPause {
                at: fmt(p.at),
                by: p.by.clone(),
                reason: p.reason.clone(),
            }),
        }
    });

    let default_policy = CompanyUpdatePolicy::default();
    for probe in &mut probes {
        let status = if probe.state != crate::storage::PROBE_ACTIVE {
            UpdateStatus::Revoked
        } else {
            let policy = policies.get(&probe.company_id).unwrap_or(&default_policy);
            let tz = zones.get(&(probe.company_id.clone(), probe.site_id.clone()));
            let facts = Facts {
                rollout: rollout.as_ref(),
                policy,
                canary: probe.canary,
                runs: probe.probe_version.as_deref(),
                failed: probe.update_failed.as_ref(),
                local: local_time(tz.map(String::as_str), now),
            };
            match plan(&facts) {
                Ok(version) => {
                    let runs = probe.probe_version.as_deref().unwrap_or_default();
                    match check_release(&version, runs, by_version.get(version.as_str()).copied()) {
                        Ok(()) => UpdateStatus::Updating { version },
                        Err(status) => status,
                    }
                }
                Err(status) => status,
            }
        };
        probe.update_status = Some(status);
    }

    let companies = storage
        .companies(&CompanyScope::All)
        .await?
        .into_iter()
        .filter(|c| !c.archived)
        .map(|c| CompanyUpdates {
            policy: policies.get(&c.id).cloned().unwrap_or_default(),
            company_id: c.id,
            company_name: c.name,
        })
        .collect();
    Ok((info, probes, companies))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UpdateWindow;
    use crate::storage::RolloutPaused;

    fn rollout(stage: RolloutStage) -> Rollout {
        Rollout {
            version: "0.3.0".into(),
            approved_by: Some("owner".into()),
            approved_at: "2026-10-09T10:00:00Z".parse().unwrap(),
            stage,
            soak_minutes: 60,
            canaries_ready_at: None,
            promoted_at: None,
            promoted_by: None,
            paused: None,
        }
    }

    fn facts<'a>(
        rollout: Option<&'a Rollout>,
        policy: &'a CompanyUpdatePolicy,
        canary: bool,
    ) -> Facts<'a> {
        Facts {
            rollout,
            policy,
            canary,
            runs: Some("0.2.0"),
            failed: None,
            local: (3, 0),
        }
    }

    #[test]
    fn a_rollout_goes_to_canaries_first_then_everyone() {
        let follow = CompanyUpdatePolicy::default();
        let canary_stage = rollout(RolloutStage::Canary);
        assert_eq!(
            plan(&facts(Some(&canary_stage), &follow, true)),
            Ok("0.3.0".into())
        );
        assert_eq!(
            plan(&facts(Some(&canary_stage), &follow, false)),
            Err(UpdateStatus::AwaitingCanaries {
                version: "0.3.0".into()
            })
        );
        let everyone = rollout(RolloutStage::All);
        assert_eq!(
            plan(&facts(Some(&everyone), &follow, false)),
            Ok("0.3.0".into())
        );
        assert_eq!(
            plan(&facts(None, &follow, false)),
            Err(UpdateStatus::NoneApproved)
        );

        let mut paused = everyone.clone();
        paused.paused = Some(RolloutPaused {
            at: paused.approved_at,
            by: None,
            reason: "canary failed".into(),
        });
        assert!(matches!(
            plan(&facts(Some(&paused), &follow, true)),
            Err(UpdateStatus::Paused { .. })
        ));

        // What it runs, what it refused, and a probe that can't say.
        let mut f = facts(Some(&everyone), &follow, false);
        f.runs = Some("0.3.0");
        assert_eq!(plan(&f), Err(UpdateStatus::UpToDate));
        let refused = UpdateFailure {
            version: "0.3.0".into(),
            error: "self-test failed".into(),
        };
        let mut f = facts(Some(&everyone), &follow, false);
        f.failed = Some(&refused);
        assert!(matches!(plan(&f), Err(UpdateStatus::Refused { .. })));
        f.runs = None;
        assert_eq!(plan(&f), Err(UpdateStatus::NoVersion));
    }

    #[test]
    fn a_company_can_be_off_pinned_or_held_to_a_window() {
        let everyone = rollout(RolloutStage::All);
        let off = CompanyUpdatePolicy {
            mode: UpdateMode::Off,
            ..Default::default()
        };
        assert_eq!(
            plan(&facts(Some(&everyone), &off, true)),
            Err(UpdateStatus::CompanyOff)
        );

        // Pinned: that release, rollout or not, paused or not, no canaries.
        let pinned = CompanyUpdatePolicy {
            mode: UpdateMode::Pin,
            version: Some("0.2.5".into()),
            window: None,
        };
        assert_eq!(plan(&facts(None, &pinned, false)), Ok("0.2.5".into()));
        let canary_stage = rollout(RolloutStage::Canary);
        assert_eq!(
            plan(&facts(Some(&canary_stage), &pinned, false)),
            Ok("0.2.5".into())
        );

        let night = CompanyUpdatePolicy {
            window: Some(UpdateWindow {
                start: "22:00".into(),
                end: "04:00".into(),
            }),
            ..Default::default()
        };
        let mut f = facts(Some(&everyone), &night, false);
        for (local, open) in [
            ((3, 0), true),
            ((22, 0), true),
            ((4, 0), false),
            ((12, 30), false),
        ] {
            f.local = local;
            assert_eq!(plan(&f).is_ok(), open, "{local:?}");
        }
        f.local = (12, 30);
        assert!(matches!(plan(&f), Err(UpdateStatus::OutsideWindow { .. })));
    }

    fn stored(version: &str, rollback_from: &[&str]) -> releases::Stored {
        releases::Stored {
            manifest: common::release::Manifest {
                format: common::release::MANIFEST_FORMAT,
                version: version.into(),
                released: "2026-10-09T10:00:00Z".parse().unwrap(),
                protocol: common::protocol::PROTOCOL_VERSION,
                artifacts: vec![],
            },
            rollback: (!rollback_from.is_empty()).then(|| common::release::Rollback {
                format: common::release::MANIFEST_FORMAT,
                version: version.into(),
                from: rollback_from.iter().map(|v| v.to_string()).collect(),
                signed: "2026-10-09T10:00:00Z".parse().unwrap(),
            }),
            dir: PathBuf::new(),
        }
    }

    #[test]
    fn an_older_release_goes_only_where_its_rollback_says() {
        assert_eq!(
            check_release("0.3.0", "0.2.0", Some(&stored("0.3.0", &[]))),
            Ok(())
        );
        assert!(matches!(
            check_release("0.2.0", "0.3.0", Some(&stored("0.2.0", &[]))),
            Err(UpdateStatus::NoRollback { .. })
        ));
        assert!(matches!(
            check_release("0.2.0", "0.3.1", Some(&stored("0.2.0", &["0.3.0"]))),
            Err(UpdateStatus::NoRollback { .. })
        ));
        assert_eq!(
            check_release("0.2.0", "0.3.0", Some(&stored("0.2.0", &["0.3.0"]))),
            Ok(())
        );
        assert!(matches!(
            check_release("0.3.0", "0.2.0", None),
            Err(UpdateStatus::Missing { .. })
        ));
    }

    fn canary(version: &str, failed: Option<&str>) -> ProbeInfo {
        ProbeInfo {
            id: "p".into(),
            name: "canary-1".into(),
            company_id: "acme".into(),
            company_name: "Acme".into(),
            site_id: "hq".into(),
            site_name: "HQ".into(),
            state: "active".into(),
            hostname: None,
            os: None,
            arch: None,
            probe_version: Some(version.into()),
            enrolled_at: String::new(),
            last_seen_at: None,
            interval_secs: 60,
            update_failed: failed.map(|v| UpdateFailure {
                version: v.into(),
                error: "rolled back: crashed".into(),
            }),
            silent: false,
            config_revision: None,
            config_rejected: None,
            config_sync: None,
            health: None,
            canary: true,
            update_status: None,
        }
    }

    #[test]
    fn the_canaries_promote_or_pause_a_rollout() {
        let now: DateTime<Utc> = "2026-10-09T12:00:00Z".parse().unwrap();
        let mut r = rollout(RolloutStage::Canary);
        assert_eq!(canary_step(&r, &[], now), Step::Promote);

        let (old, new) = (canary("0.2.0", None), canary("0.3.0", None));
        assert_eq!(canary_step(&r, &[&old, &new], now), Step::Wait);
        assert_eq!(canary_step(&r, &[&new, &new], now), Step::MarkReady);
        r.canaries_ready_at = Some(now - Duration::minutes(59));
        assert_eq!(canary_step(&r, &[&new], now), Step::Wait);
        r.canaries_ready_at = Some(now - Duration::minutes(60));
        assert_eq!(canary_step(&r, &[&new], now), Step::Promote);
        assert_eq!(canary_step(&r, &[&new, &old], now), Step::MarkNotReady);

        let failed = canary("0.2.0", Some("0.3.0"));
        assert!(matches!(
            canary_step(&r, &[&new, &failed], now),
            Step::Pause(reason) if reason.contains("canary-1") && reason.contains("crashed")
        ));
        // A failure of another version is old news.
        let old_failure = canary("0.3.0", Some("0.2.9"));
        assert_eq!(canary_step(&r, &[&old_failure], now), Step::Promote);
    }

    #[test]
    fn only_active_canaries_of_following_companies_count() {
        let mut revoked = canary("0.2.0", None);
        revoked.state = "revoked".into();
        let mut elsewhere = canary("0.2.0", None);
        elsewhere.company_id = "pinned-co".into();
        let mut plain = canary("0.2.0", None);
        plain.canary = false;
        let counted = canary("0.2.0", None);
        let probes = vec![revoked, elsewhere, plain, counted];
        let policies = HashMap::from([(
            "pinned-co".to_string(),
            CompanyUpdatePolicy {
                mode: UpdateMode::Off,
                ..Default::default()
            },
        )]);
        let found = canaries(&probes, &policies);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].company_id, "acme");
    }

    #[test]
    fn local_time_follows_the_site_zone() {
        let now: DateTime<Utc> = "2026-01-15T23:30:00Z".parse().unwrap();
        assert_eq!(local_time(Some("Europe/Madrid"), now), (0, 30));
        assert_eq!(local_time(Some("Not/AZone"), now), (23, 30));
        assert_eq!(local_time(None, now), (23, 30));
    }
}
