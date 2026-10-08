//! The daily digest (M3-09, D-71): set on the Alerts page, sent once a day
//! at its local hour to e-mail channels, also on a quiet day; "Send digest
//! now"; a removed channel leaves it.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use hub_lib::model::{AlertsOverview, ChannelInfo, TestResult};
use hub_lib::service::digest;
use hub_lib::service::notify::payload::Outgoing;
use serde_json::{Value, json};
use support::*;

/// The mail server, an e-mail channel and a webhook; their IDs.
async fn setup(hub: &TestHub, cookie: &str) -> (i64, i64) {
    let settings = json!({ "smtp_server": "smtp.example.com", "smtp_port": 587,
        "smtp_security": "starttls", "smtp_from": "hub@example.com" });
    let response = hub
        .post_json("/api/admin/alerts/settings", Some(cookie), settings)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let mut ids = Vec::new();
    for body in [
        json!({ "name": "Ops mail", "kind": "email", "enabled": true, "target": "ops@example.com" }),
        json!({ "name": "Hook", "kind": "webhook", "enabled": true,
                "target": "https://example.com/hook", "secret": "key-0123456789abcdef" }),
    ] {
        let response = hub
            .post_json("/api/admin/alerts/channels", Some(cookie), body)
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        ids.push(response.json::<ChannelInfo>().id);
    }
    (ids[0], ids[1])
}

async fn save(hub: &TestHub, cookie: &str, body: Value) -> (StatusCode, String) {
    let response = hub
        .post_json("/api/admin/alerts/digest", Some(cookie), body)
        .await;
    (response.status, response.text())
}

/// The e-mails sent so far: (to, subject, body).
fn mails(hub: &TestHub) -> Vec<(Vec<String>, String, String)> {
    hub.transport
        .sent()
        .into_iter()
        .filter_map(|o| match o {
            Outgoing::Email {
                to, subject, body, ..
            } => Some((to, subject, body)),
            Outgoing::Http { .. } => None,
        })
        .collect()
}

async fn sweep(hub: &TestHub) {
    digest::sweep(hub.store(), &hub.delivery(), hub.now())
        .await
        .unwrap();
}

#[tokio::test]
async fn the_digest_goes_once_a_day_at_its_local_hour() {
    let hub = TestHub::new(&[]).await; // 12:00 UTC: 13:00 in Madrid
    let owner = hub.login().await;
    let (mail, _) = setup(&hub, &owner).await;
    let (status, text) = save(
        &hub,
        &owner,
        json!({ "hour": 14, "timezone": "Europe/Madrid", "channels": [mail] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    sweep(&hub).await;
    assert!(mails(&hub).is_empty(), "13:00 there: not yet");

    hub.clock.advance(Duration::hours(1));
    sweep(&hub).await;
    let sent = mails(&hub);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, ["ops@example.com"]);
    assert_eq!(sent[0].1, "[BLENTINEL] Daily digest 2026-03-10: all well");
    assert!(
        sent[0]
            .2
            .starts_with("Blentinel daily digest, 2026-03-10 14:00 Europe/Madrid")
    );

    // Once that day, also after a restart (the date is in the database).
    for _ in 0..3 {
        hub.clock.advance(Duration::hours(3));
        sweep(&hub).await;
    }
    assert_eq!(mails(&hub).len(), 1);

    // The next day, at its hour (23:00 there now: 15 hours to 14:00).
    hub.clock.advance(Duration::hours(15));
    sweep(&hub).await;
    assert_eq!(mails(&hub).len(), 2);
    let alerts: AlertsOverview = hub.get_json("/api/admin/alerts").await;
    assert_eq!(alerts.digest_last_sent.as_deref(), Some("2026-03-11"));
}

#[tokio::test]
async fn the_digest_says_what_is_open_and_who_is_on_it() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let (mail, _) = setup(&hub, &owner).await;
    save(
        &hub,
        &owner,
        json!({ "hour": 0, "timezone": "UTC", "channels": [mail] }),
    )
    .await;
    for _ in 0..2 {
        hub.evaluate_report(&probe, vec![ping("NAS", "10.0.0.20", false)])
            .await;
    }
    hub.clock.advance(Duration::minutes(30));
    // Stored results for uptime: one check up, one down.
    let report = probe.signed_report(
        "acme",
        hub.now(),
        vec![
            ping("NAS", "10.0.0.20", false),
            ping("GW", "10.0.0.1", true),
        ],
    );
    hub.seed_report(&report).await;

    let response = hub
        .post_json("/api/admin/alerts/digest/send", Some(&owner), json!({}))
        .await;
    assert_eq!(
        response.json::<TestResult>(),
        TestResult {
            ok: true,
            error: None
        }
    );

    let (_, subject, body) = mails(&hub).pop().unwrap();
    assert_eq!(
        subject,
        "[BLENTINEL] Daily digest 2026-03-10: 1 open incident (1 not acknowledged)"
    );
    assert!(
        body.contains("- DOWN acme › hq-site › hq › NAS: for 30 min, NOT ACKNOWLEDGED"),
        "{body}"
    );
    assert!(
        body.contains("UPTIME, LAST 24 HOURS\n- acme: 50.0 %"),
        "{body}"
    );
    // "Send digest now" isn't the day's digest.
    let alerts: AlertsOverview = hub.get_json("/api/admin/alerts").await;
    assert_eq!(alerts.digest_last_sent, None);
}

#[tokio::test]
async fn the_digest_goes_to_e_mail_channels_in_a_real_time_zone() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let (mail, hook) = setup(&hub, &owner).await;

    for (body, reason) in [
        (
            json!({ "hour": 7, "timezone": "UTC", "channels": [hook] }),
            "e-mail channels",
        ),
        (
            json!({ "hour": 7, "timezone": "Mars/Olympus", "channels": [mail] }),
            "not a time zone",
        ),
        (
            json!({ "hour": 24, "timezone": "UTC", "channels": [mail] }),
            "0 to 23",
        ),
    ] {
        let (status, text) = save(&hub, &owner, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        assert!(text.contains(reason), "{text}");
    }

    // Without channels it is off, and "Send digest now" says so.
    let response = hub
        .post_json("/api/admin/alerts/digest/send", Some(&owner), json!({}))
        .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);

    // Removing its channel takes it out of the digest.
    save(
        &hub,
        &owner,
        json!({ "hour": 7, "timezone": "UTC", "channels": [mail] }),
    )
    .await;
    let response = hub
        .post_json(
            &format!("/api/admin/alerts/channels/{mail}/delete"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let alerts: AlertsOverview = hub.get_json("/api/admin/alerts").await;
    assert!(alerts.digest.channels.is_empty());
}
