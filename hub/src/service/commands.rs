//! The command channel (M2-06, proposal 0003 §4.3, D-56).
//!
//! A user queues a command for a probe; it rides in the signed answer to
//! the probe's next report, again in every answer until its result comes
//! back with a later report, or it expires. The hub never reaches out to
//! the probe (FP-1): a command also goes in the answer to the probe's
//! command poll, which the hub holds until one is queued (`service::poll`,
//! D-76), so it usually arrives within seconds.
//!
//! Who may queue or cancel: `ManageCompanies` within the user's company
//! scope, like changing the probe's checks; seeing them: anyone who sees
//! the probe.

use chrono::{DateTime, Duration, Utc};
use common::commands::{Command, CommandResult, CommandSpec, MAX_COMMANDS_PER_MESSAGE};
use rand::RngCore;
use rand::rngs::OsRng;

use crate::model::{CommandInfo, CommandState, CompanyScope};
use crate::service::probes::{ProbeError, ProbeResult, scoped_record};
use crate::service::scope;
use crate::storage::{CommandRecord, ProbeRecord, Storage};

/// How many of a probe's commands the API lists.
pub const LISTED: u32 = 20;

/// The shortest time a command waits for its probe.
const MIN_TTL: Duration = Duration::minutes(15);

/// How many report intervals a command waits for its probe, if longer.
const TTL_INTERVALS: i32 = 3;

fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// How long a command for a probe reporting every `interval_secs` waits:
/// a few intervals, so a probe that missed a report still gets it, but not
/// so long that "run now" happens hours later.
pub fn time_to_live(interval_secs: u32) -> Duration {
    MIN_TTL.max(Duration::seconds(i64::from(interval_secs)) * TTL_INTERVALS)
}

/// The state to show at `now`: a pending command past its expiry has expired.
pub fn state_at(command: &CommandRecord, now: DateTime<Utc>) -> CommandState {
    if command.state.is_pending() && command.expires_at <= now {
        CommandState::Expired
    } else {
        command.state
    }
}

/// The command as the API shows it at `now`.
pub fn to_info(command: &CommandRecord, now: DateTime<Utc>) -> CommandInfo {
    CommandInfo {
        id: command.id.clone(),
        probe_id: command.probe_id.clone(),
        kind: command.spec.kind().to_string(),
        created_by: command.created_by.clone(),
        created_at: fmt_time(command.created_at),
        expires_at: fmt_time(command.expires_at),
        state: state_at(command, now),
        delivered_at: command.delivered_at.map(fmt_time),
        completed_at: command.completed_at.map(fmt_time),
        result: command.result.clone(),
    }
}

/// A new, random command ID (16 bytes, hex).
fn new_command_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// The probe's latest commands, newest first.
pub async fn probe_commands(
    store: &dyn Storage,
    scope: &CompanyScope,
    probe_id: &str,
    now: DateTime<Utc>,
) -> ProbeResult<Vec<CommandInfo>> {
    scoped_record(store, scope, probe_id).await?;
    let commands = store.probe_commands(probe_id, LISTED).await?;
    Ok(commands.iter().map(|c| to_info(c, now)).collect())
}

/// Queue a command for an active probe of one of the user's companies. The
/// same command already waiting is returned instead of queuing another, so
/// clicking twice runs it once. `true` if it was queued now.
pub async fn queue(
    store: &dyn Storage,
    scope: &CompanyScope,
    probe_id: &str,
    spec: &CommandSpec,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<(CommandRecord, bool)> {
    let record = scoped_record(store, scope, probe_id).await?;
    if *spec == CommandSpec::Unknown {
        return Err(ProbeError::Invalid("unknown command kind".into()));
    }
    if !record.is_active() {
        return Err(ProbeError::Invalid(
            "the probe is revoked: it takes no commands".into(),
        ));
    }
    // A look at the network, only where the company allows it (D-101).
    if *spec == CommandSpec::Discover
        && !crate::service::discovery::allowed(store, &record.company_id).await?
    {
        return Err(ProbeError::Forbidden(
            crate::service::discovery::NOT_ALLOWED.into(),
        ));
    }
    let pending = store
        .pending_commands(probe_id, now, MAX_COMMANDS_PER_MESSAGE as u32)
        .await?;
    if let Some(same) = pending.into_iter().find(|c| c.spec == *spec) {
        return Ok((same, false));
    }
    let command = CommandRecord {
        id: new_command_id(),
        probe_id: record.id.clone(),
        company_id: record.company_id.clone(),
        spec: spec.clone(),
        created_by: Some(by.to_string()),
        created_at: now,
        expires_at: now + time_to_live(record.interval_secs),
        state: CommandState::Queued,
        delivered_at: None,
        completed_at: None,
        result: None,
    };
    store.create_command(&command).await?;
    Ok((command, true))
}

/// Cancel a command that hasn't been sent yet. Returns it, cancelled.
pub async fn cancel(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
    now: DateTime<Utc>,
) -> ProbeResult<CommandRecord> {
    let command = store.command(id).await?.ok_or(ProbeError::NotFound)?;
    scope::require(scope, &command.company_id).map_err(|_| ProbeError::NotFound)?;
    if !store.cancel_command(id, now).await? {
        return Err(ProbeError::Invalid(
            "only a command that hasn't been sent yet can be cancelled".into(),
        ));
    }
    Ok(CommandRecord {
        state: CommandState::Cancelled,
        completed_at: Some(now),
        ..command
    })
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// Record what a report says came of its probe's commands. Results for
/// another probe's commands, or for ones already done, are ignored.
pub async fn record_results(
    store: &dyn Storage,
    probe_id: &str,
    results: &[CommandResult],
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    for result in results.iter().take(MAX_COMMANDS_PER_MESSAGE) {
        let state = if result.ok {
            CommandState::Succeeded
        } else {
            CommandState::Failed
        };
        let message = common::commands::truncate(&result.message);
        if !store
            .complete_command(probe_id, &result.id, state, &message, now)
            .await?
        {
            tracing::debug!(command = %result.id, "Ignored the result of a command that isn't pending");
        }
    }
    Ok(())
}

/// The commands to put in the answer to a report from `probe`, marked
/// delivered. Every probe the hub accepts (v6 and later) takes commands.
pub async fn to_send(
    store: &dyn Storage,
    probe: &ProbeRecord,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<Command>> {
    let pending = store
        .pending_commands(&probe.id, now, MAX_COMMANDS_PER_MESSAGE as u32)
        .await?;
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = pending.iter().map(|c| c.id.clone()).collect();
    store.mark_commands_delivered(&ids, now).await?;
    Ok(pending
        .into_iter()
        .map(|c| Command {
            id: c.id,
            spec: c.spec,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(state: CommandState, expires_at: &str) -> CommandRecord {
        CommandRecord {
            id: "c".into(),
            probe_id: "p".into(),
            company_id: "acme".into(),
            spec: CommandSpec::RunChecks,
            created_by: None,
            created_at: "2026-10-04T10:00:00Z".parse().unwrap(),
            expires_at: expires_at.parse().unwrap(),
            state,
            delivered_at: None,
            completed_at: None,
            result: None,
        }
    }

    #[test]
    fn a_command_waits_a_few_intervals_but_at_least_a_quarter_hour() {
        assert_eq!(time_to_live(60), Duration::minutes(15));
        assert_eq!(time_to_live(600), Duration::minutes(30));
        assert_eq!(time_to_live(3600), Duration::hours(3));
    }

    #[test]
    fn only_a_pending_command_expires() {
        let now = "2026-10-04T11:00:00Z".parse().unwrap();
        let past = "2026-10-04T10:15:00Z";
        let future = "2026-10-04T12:00:00Z";
        assert_eq!(
            state_at(&command(CommandState::Queued, past), now),
            CommandState::Expired
        );
        assert_eq!(
            state_at(&command(CommandState::Delivered, past), now),
            CommandState::Expired
        );
        assert_eq!(
            state_at(&command(CommandState::Delivered, future), now),
            CommandState::Delivered
        );
        assert_eq!(
            state_at(&command(CommandState::Succeeded, past), now),
            CommandState::Succeeded
        );
    }
}
