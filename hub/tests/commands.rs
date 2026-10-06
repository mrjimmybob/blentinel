//! The command channel (M2-06, D-56): queuing commands through the API, the
//! hub sending them in its signed answer to a report until the probe reports
//! their result, expiry, cancelling.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::ReportAck;
use common::commands::{CommandResult, CommandSpec};
use hub_lib::model::{AuditEntry, CommandInfo, CommandState};
use serde_json::json;
use support::*;

async fn run_now(hub: &TestHub, cookie: &str, probe: &TestProbe) -> TestResponse {
    hub.post_json(
        &format!("/api/probes/{}/commands", probe.id),
        Some(cookie),
        json!({ "kind": "run_checks" }),
    )
    .await
}

async fn queued(hub: &TestHub, cookie: &str, probe: &TestProbe) -> CommandInfo {
    let response = run_now(hub, cookie, probe).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

async fn commands_of(hub: &TestHub, probe: &TestProbe) -> Vec<CommandInfo> {
    hub.get_json(&format!("/api/probes/{}/commands", probe.id))
        .await
}

async fn state_of(hub: &TestHub, probe: &TestProbe, id: &str) -> CommandInfo {
    commands_of(hub, probe)
        .await
        .into_iter()
        .find(|c| c.id == id)
        .expect("the command is listed")
}

/// Report from `probe` a minute later, with `results`; the hub's answer.
async fn report(hub: &TestHub, probe: &TestProbe, results: Vec<CommandResult>) -> ReportAck {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.command_results = results;
    probe.sign(&mut report);
    let (status, ack) = hub.send_report_for_ack(probe, &report).await;
    assert_eq!(status, StatusCode::OK);
    ack.expect("a trusted ack")
}

fn ids(ack: &ReportAck) -> Vec<&str> {
    ack.commands.iter().map(|c| c.id.as_str()).collect()
}

#[tokio::test]
async fn run_now_rides_in_every_answer_until_its_result_comes_back() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, vec![]).await; // enrolled and reporting

    let command = queued(&hub, &owner, &probe).await;
    assert_eq!(command.kind, "run_checks");
    assert_eq!(command.state, CommandState::Queued);
    assert_eq!(command.created_by.as_deref(), Some(OWNER));

    let ack = report(&hub, &probe, vec![]).await;
    assert_eq!(ids(&ack), [command.id.as_str()]);
    assert_eq!(ack.commands[0].spec, CommandSpec::RunChecks);
    let delivered = state_of(&hub, &probe, &command.id).await;
    assert_eq!(delivered.state, CommandState::Delivered);
    assert!(delivered.delivered_at.is_some());

    // Sent again until the result arrives (the probe skips what it ran).
    let ack = report(&hub, &probe, vec![]).await;
    assert_eq!(ids(&ack), [command.id.as_str()]);

    let done = CommandResult::new(&command.id, true, "ran 2 checks");
    let ack = report(&hub, &probe, vec![done]).await;
    assert!(ack.commands.is_empty(), "done: not sent again");
    let succeeded = state_of(&hub, &probe, &command.id).await;
    assert_eq!(succeeded.state, CommandState::Succeeded);
    assert_eq!(succeeded.result.as_deref(), Some("ran 2 checks"));
    assert!(succeeded.completed_at.is_some());

    // A late copy of the result changes nothing.
    let again = CommandResult::new(&command.id, false, "late");
    report(&hub, &probe, vec![again]).await;
    let still = state_of(&hub, &probe, &command.id).await;
    assert_eq!(still.state, CommandState::Succeeded);
}

#[tokio::test]
async fn a_failed_command_shows_the_probes_reason() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, vec![]).await;
    let command = queued(&hub, &owner, &probe).await;
    report(&hub, &probe, vec![]).await;

    let failed = CommandResult::new(&command.id, false, "doesn't know this command");
    report(&hub, &probe, vec![failed]).await;
    let shown = state_of(&hub, &probe, &command.id).await;
    assert_eq!(shown.state, CommandState::Failed);
    assert_eq!(shown.result.as_deref(), Some("doesn't know this command"));
}

#[tokio::test]
async fn clicking_twice_queues_one_command() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, vec![]).await;

    let first = queued(&hub, &owner, &probe).await;
    let second = queued(&hub, &owner, &probe).await;
    assert_eq!(first.id, second.id);
    assert_eq!(commands_of(&hub, &probe).await.len(), 1);

    let audit: Vec<AuditEntry> = hub.get_json("/api/audit?action=command").await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0].action, "command.queue");
    assert_eq!(audit[0].target_id.as_deref(), Some(probe.id.as_str()));

    // Once delivered, it still counts as waiting.
    report(&hub, &probe, vec![]).await;
    assert_eq!(queued(&hub, &owner, &probe).await.id, first.id);
}

#[tokio::test]
async fn only_a_command_not_sent_yet_can_be_cancelled() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, vec![]).await;

    let command = queued(&hub, &owner, &probe).await;
    let cancel = |id: String| {
        let (hub, owner) = (&hub, owner.clone());
        async move {
            hub.post_json(
                &format!("/api/commands/{id}/cancel"),
                Some(&owner),
                json!({}),
            )
            .await
        }
    };
    let response = cancel(command.id.clone()).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json::<CommandInfo>().state,
        CommandState::Cancelled
    );
    assert!(report(&hub, &probe, vec![]).await.commands.is_empty());

    let sent = queued(&hub, &owner, &probe).await;
    assert_ne!(sent.id, command.id, "a cancelled command isn't waiting");
    report(&hub, &probe, vec![]).await;
    assert_eq!(
        cancel(sent.id.clone()).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        cancel("nonexistent".into()).await.status,
        StatusCode::NOT_FOUND
    );

    let audit: Vec<AuditEntry> = hub.get_json("/api/audit?action=command.cancel").await;
    assert_eq!(audit.len(), 1);
}

#[tokio::test]
async fn a_command_the_probe_didnt_pick_up_in_time_expires() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, vec![]).await;

    let command = queued(&hub, &owner, &probe).await;
    // A 60 s interval: it waits 15 minutes.
    hub.clock.advance(Duration::minutes(16));
    assert_eq!(
        state_of(&hub, &probe, &command.id).await.state,
        CommandState::Expired
    );
    assert!(report(&hub, &probe, vec![]).await.commands.is_empty());
    // Expired isn't waiting: a new click queues a new command.
    assert_ne!(queued(&hub, &owner, &probe).await.id, command.id);
}

#[tokio::test]
async fn a_probe_cant_report_on_another_probes_command() {
    let (hq, branch) = (TestProbe::new("hq"), TestProbe::new("branch"));
    let hub = TestHub::new(&[&hq, &branch]).await;
    let owner = hub.login().await;
    report(&hub, &hq, vec![]).await;
    report(&hub, &branch, vec![]).await;

    let command = queued(&hub, &owner, &hq).await;
    assert!(report(&hub, &branch, vec![]).await.commands.is_empty());
    let forged = CommandResult::new(&command.id, true, "done, honest");
    report(&hub, &branch, vec![forged]).await;
    assert_eq!(
        state_of(&hub, &hq, &command.id).await.state,
        CommandState::Queued
    );
}

#[tokio::test]
async fn unknown_kinds_and_revoked_probes_are_refused() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, vec![]).await;

    let bogus = hub
        .post_json(
            &format!("/api/probes/{}/commands", probe.id),
            Some(&owner),
            json!({ "kind": "format_disk" }),
        )
        .await;
    assert_eq!(bogus.status, StatusCode::BAD_REQUEST, "{}", bogus.text());

    let missing = hub
        .post_json(
            "/api/probes/nonexistent/commands",
            Some(&owner),
            json!({ "kind": "run_checks" }),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let revoked = hub
        .post_json(
            &format!("/api/probes/{}/revoke", probe.id),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert_eq!(
        run_now(&hub, &owner, &probe).await.status,
        StatusCode::BAD_REQUEST
    );
}
