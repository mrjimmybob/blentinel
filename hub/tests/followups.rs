//! Reminders and escalation (M3-06, D-69): while an incident is open and
//! nobody acknowledged it, its rules remind their channels and escalate it
//! once; acknowledging, recovery, silences and windows stop them; the
//! escalation's channels hear the recovery. Worked out from the database,
//! so a second sweep at the same time sends nothing more.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::{CheckId, CheckSpec};
use hub_lib::model::{
    AlertKind, AlertsOverview, ChannelInfo, IncidentDetail, IncidentEventKind, IncidentInfo,
    RouteRule,
};
use hub_lib::service::notify::{IncidentRef, Notification, Nudge};
use hub_lib::service::{followups, silences};
use serde_json::{Value, json};
use support::*;

/// A webhook channel called `name`; its ID.
async fn channel(hub: &TestHub, cookie: &str, name: &str) -> i64 {
    let body = json!({ "name": name, "kind": "webhook", "enabled": true,
        "target": format!("https://example.com/{name}"), "secret": "key-0123456789abcdef" });
    let response = hub
        .post_json("/api/admin/alerts/channels", Some(cookie), body)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json::<ChannelInfo>().id
}

async fn save_rules(hub: &TestHub, cookie: &str, rules: Value) -> (StatusCode, String) {
    let response = hub
        .post_json(
            "/api/admin/alerts/routes",
            Some(cookie),
            json!({ "rules": rules }),
        )
        .await;
    (response.status, response.text())
}

/// The on-call channel, the manager's, and a rule: everything to on-call,
/// reminded every 30 minutes, escalated to the manager after 60.
async fn setup(hub: &TestHub) -> (i64, i64) {
    let owner = hub.login().await;
    let on_call = channel(hub, &owner, "on-call").await;
    let manager = channel(hub, &owner, "manager").await;
    let rule = RouteRule {
        channels: vec![on_call],
        remind_every_minutes: Some(30),
        escalate_after_minutes: Some(60),
        escalate_to: vec![manager],
        ..RouteRule::default()
    };
    let (status, text) = save_rules(hub, &owner, json!([rule])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    (on_call, manager)
}

fn nas(up: bool) -> Vec<Res> {
    vec![ping("NAS", "10.0.0.20", up)]
}

/// The NAS goes down (its alert at the second result); the incident's ID.
async fn outage(hub: &TestHub, probe: &TestProbe) -> i64 {
    for _ in 0..2 {
        hub.evaluate_report(probe, nas(false)).await;
    }
    let open: Vec<IncidentInfo> = hub.get_json("/api/incidents").await;
    assert_eq!(open.len(), 1);
    open[0].id
}

/// Sweep `minutes` later; the reminders and escalations it sent.
async fn sweep_after(hub: &TestHub, minutes: i64) -> Vec<Notification> {
    hub.clock.advance(Duration::minutes(minutes));
    let before = hub.notifier.sent().len();
    followups::sweep(hub.store(), hub.notifier.as_ref(), hub.now())
        .await
        .unwrap();
    hub.notifier.sent()[before..].to_vec()
}

#[tokio::test]
async fn an_unacknowledged_outage_is_reminded_then_escalated_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let (on_call, manager) = setup(&hub).await;
    let id = outage(&hub, &probe).await;

    assert!(sweep_after(&hub, 29).await.is_empty(), "not yet");

    let sent = sweep_after(&hub, 1).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].nudge,
        Some(Nudge::Reminder {
            count: 1,
            channels: vec![on_call]
        })
    );
    assert!(
        sent[0].subject().ends_with("NAS STILL DOWN"),
        "{}",
        sent[0].subject()
    );
    assert!(
        sent[0]
            .text(None)
            .contains("Open:   for 30 min, not acknowledged (reminder 1)")
    );
    assert!(sweep_after(&hub, 0).await.is_empty(), "once per interval");

    // An hour after it opened: the second reminder, and the escalation.
    let sent = sweep_after(&hub, 30).await;
    let nudges: Vec<_> = sent.iter().map(|n| n.nudge.clone().unwrap()).collect();
    assert_eq!(
        nudges,
        [
            Nudge::Escalation {
                after_minutes: 60,
                channels: vec![manager]
            },
            Nudge::Reminder {
                count: 2,
                channels: vec![on_call]
            },
        ]
    );
    assert!(
        sent[0]
            .subject()
            .ends_with("NAS DOWN (escalated: not acknowledged for 1 h)"),
        "{}",
        sent[0].subject()
    );
    assert_eq!(sent[0].incident.unwrap().escalated, Some(AlertKind::Down));

    // Escalated once; reminded on.
    let sent = sweep_after(&hub, 30).await;
    assert!(matches!(
        sent[..],
        [Notification {
            nudge: Some(Nudge::Reminder { count: 3, .. }),
            ..
        }]
    ));

    let detail: IncidentDetail = hub.get_json(&format!("/api/incidents/{id}")).await;
    assert_eq!(
        (
            detail.incident.reminders,
            detail.incident.escalated_at.is_some()
        ),
        (3, true)
    );
    let kinds: Vec<_> = detail.events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            IncidentEventKind::Opened,
            IncidentEventKind::Reminded,
            IncidentEventKind::Escalated,
            IncidentEventKind::Reminded,
            IncidentEventKind::Reminded,
        ]
    );
    assert_eq!(
        detail.events[2].text,
        "Not acknowledged for 1 h: escalated to manager"
    );
    assert_eq!(detail.events[1].text, "Reminder 1 to on-call");
}

#[tokio::test]
async fn acknowledging_stops_reminders_and_escalation() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    setup(&hub).await;
    let id = outage(&hub, &probe).await;
    let owner = hub.login().await;

    let response = hub
        .post_json(
            &format!("/api/incidents/{id}/acknowledge"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);

    assert!(sweep_after(&hub, 120).await.is_empty());
}

#[tokio::test]
async fn recovery_stops_them_and_reaches_the_escalation_too() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let (on_call, manager) = setup(&hub).await;
    outage(&hub, &probe).await;
    sweep_after(&hub, 60).await;
    for _ in 0..2 {
        hub.evaluate_report(&probe, nas(true)).await;
    }

    assert!(sweep_after(&hub, 120).await.is_empty(), "nothing open");
    let recovered = hub.notifier.sent().pop().unwrap();
    assert_eq!(recovered.kind, AlertKind::Recovered);
    assert_eq!(recovered.incident.unwrap().escalated, Some(AlertKind::Down));

    // Delivered: the rule's channel and the escalation's.
    dispatch(&hub.dispatcher(1), recovered).await;
    assert_eq!(
        hub.delivered_urls(),
        ["https://example.com/on-call", "https://example.com/manager"],
        "channels {on_call} then {manager}"
    );
}

#[tokio::test]
async fn a_reminder_goes_to_its_channels_only() {
    let hub = TestHub::new(&[]).await;
    let (_, manager) = setup(&hub).await;
    let n = Notification {
        test: false,
        incident: Some(IncidentRef {
            id: 1,
            opened_at: hub.now(),
            escalated: None,
        }),
        nudge: Some(Nudge::Reminder {
            count: 1,
            channels: vec![manager],
        }),
        ..Notification::sample(hub.now())
    };

    dispatch(&hub.dispatcher(1), n).await;

    assert_eq!(hub.delivered_urls(), ["https://example.com/manager"]);
}

#[tokio::test]
async fn an_outage_nobody_was_told_about_is_never_reminded() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    setup(&hub).await;
    let key = CheckId(
        hub.add_check(
            &probe,
            "NAS",
            CheckSpec::Ping {
                host: "10.0.0.20".into(),
            },
        )
        .await,
    );
    silences::silence_check(hub.store(), &key, "known", Some(1), hub.now())
        .await
        .unwrap();
    outage(&hub, &probe).await;

    // Even after the silence ran out.
    assert!(sweep_after(&hub, 180).await.is_empty());
}

#[tokio::test]
async fn a_silence_pauses_reminders_until_it_ends() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    setup(&hub).await;
    outage(&hub, &probe).await;
    silences::silence_check(
        hub.store(),
        &CheckId(check_id(&probe.id, "NAS")),
        "working on it",
        Some(1),
        hub.now(),
    )
    .await
    .unwrap();

    assert!(sweep_after(&hub, 59).await.is_empty(), "silenced");
    let sent = sweep_after(&hub, 2).await;
    assert_eq!(
        sent.len(),
        2,
        "the silence is over: reminder and escalation"
    );
}

#[tokio::test]
async fn reminder_and_escalation_settings_are_checked() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let a = channel(&hub, &owner, "a").await;
    let b = channel(&hub, &owner, "b").await;
    let base = json!({ "name": "R", "channels": [a] });
    let with = |extra: Value| {
        let mut rule = base.clone();
        rule.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!([rule])
    };

    for (extra, reason) in [
        (
            json!({ "remind_every_minutes": 5 }),
            "remind every 15-10080 minutes",
        ),
        (
            json!({ "escalate_after_minutes": 30 }),
            "choose where it escalates to",
        ),
        (
            json!({ "escalate_to": [b] }),
            "say after how long it escalates",
        ),
        (
            json!({ "escalate_after_minutes": 2, "escalate_to": [b] }),
            "escalate after 5-1440 minutes",
        ),
        (
            json!({ "escalate_after_minutes": 30, "escalate_to": [99] }),
            "doesn't exist",
        ),
    ] {
        let (status, text) = save_rules(&hub, &owner, with(extra)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        assert!(text.contains(reason), "{text}");
    }

    let (status, text) = save_rules(
        &hub,
        &owner,
        with(
            json!({ "remind_every_minutes": 60, "escalate_after_minutes": 30, "escalate_to": [b] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // Removing the escalation's only channel removes the escalation.
    let response = hub
        .post_json(
            &format!("/api/admin/alerts/channels/{b}/delete"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let alerts: AlertsOverview = hub.get_json("/api/admin/alerts").await;
    let rule = &alerts.rules[0];
    assert_eq!(
        (
            rule.remind_every_minutes,
            rule.escalate_after_minutes,
            rule.escalate_to.len()
        ),
        (Some(60), None, 0)
    );
}
