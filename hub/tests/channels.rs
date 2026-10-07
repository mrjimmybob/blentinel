//! Alert channels and routing (M3-03, D-66): managing channels over the API
//! (secrets are write-only), "Send test", routing rules, retries, and a
//! signed webhook delivered over a real socket.
#![cfg(feature = "ssr")]

mod support;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use hmac::{Hmac, Mac};
use hub_lib::model::{AlertKind, AlertsOverview, ChannelInfo, Role, RouteRule, TestResult};
use hub_lib::service::notify::payload::Outgoing;
use hub_lib::service::notify::transport::NetTransport;
use hub_lib::service::notify::{Delivery, Dispatcher, Notification, channels};
use serde_json::{Value, json};
use sha2::Sha256;
use support::*;

const SECRET: &str = "webhook-signing-key-0123456789";

async fn overview(hub: &TestHub) -> AlertsOverview {
    hub.get_json("/api/admin/alerts").await
}

/// Create a channel as the Owner; its ID.
async fn create(hub: &TestHub, cookie: &str, body: Value) -> i64 {
    let response = hub
        .post_json("/api/admin/alerts/channels", Some(cookie), body)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json::<ChannelInfo>().id
}

fn webhook(name: &str) -> Value {
    json!({ "name": name, "kind": "webhook", "enabled": true,
            "target": "https://example.com/hook", "secret": SECRET })
}

fn rule(kinds: Vec<AlertKind>, channels: Vec<i64>, stop: bool) -> RouteRule {
    RouteRule {
        name: String::new(),
        companies: Vec::new(),
        sites: Vec::new(),
        kinds,
        check_kinds: Vec::new(),
        channels,
        stop,
    }
}

#[tokio::test]
async fn a_fresh_hub_has_no_channels_and_sends_nothing() {
    let hub = TestHub::new(&[]).await;
    hub.login().await;

    let alerts = overview(&hub).await;

    assert!(alerts.channels.is_empty() && alerts.rules.is_empty());
    assert_eq!(alerts.settings.smtp_port, 587);
}

#[tokio::test]
async fn a_channels_secret_is_sealed_and_never_shown() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let id = create(&hub, &owner, webhook("Ops")).await;
    let body = hub.get("/api/admin/alerts", Some(&owner)).await.text();
    assert!(!body.contains(SECRET), "the API never returns a secret");
    let stored = hub.store().channel(id).await.unwrap().unwrap();
    let sealed = stored.secret_sealed.expect("a sealed secret");
    assert!(!sealed.contains(SECRET), "sealed in the database");

    // A change without a secret keeps it.
    let mut change = webhook("Ops team");
    change["secret"] = Value::Null;
    let response = hub
        .post_json(
            &format!("/api/admin/alerts/channels/{id}"),
            Some(&owner),
            change,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let channel = &overview(&hub).await.channels[0];
    assert_eq!(
        (channel.name.as_str(), channel.secret_set),
        ("Ops team", true)
    );

    let audit: Vec<String> =
        sqlx::query_scalar("SELECT details FROM audit_log WHERE action LIKE 'channel.%'")
            .fetch_all(hub.pool())
            .await
            .unwrap();
    assert_eq!(audit.len(), 2);
    assert!(audit.iter().all(|d| !d.contains(SECRET)), "{audit:?}");
}

#[tokio::test]
async fn a_bad_channel_is_refused_with_the_reason() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    for (body, reason) in [
        (
            json!({ "name": "", "kind": "email", "enabled": true, "target": "ops@example.com" }),
            "name",
        ),
        (
            json!({ "name": "Mail", "kind": "email", "enabled": true, "target": "not an address" }),
            "e-mail address",
        ),
        (
            json!({ "name": "Hook", "kind": "webhook", "enabled": true, "target": "https://example.com" }),
            "needed",
        ),
        (
            json!({ "name": "Slack", "kind": "slack", "enabled": true, "target": "", "secret": "http://example.com" }),
            "https://",
        ),
    ] {
        let response = hub
            .post_json("/api/admin/alerts/channels", Some(&owner), body)
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(response.error_code(), "invalid_alerts");
        assert!(response.text().contains(reason), "{}", response.text());
    }
}

#[tokio::test]
async fn technicians_cant_see_or_change_channels() {
    let hub = TestHub::new(&[]).await;
    let technician = hub.login_as(Role::Technician).await;

    assert_eq!(
        hub.get("/api/admin/alerts", Some(&technician)).await.status,
        StatusCode::FORBIDDEN
    );
    let response = hub
        .post_json(
            "/api/admin/alerts/channels",
            Some(&technician),
            webhook("Mine"),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn removing_a_channel_removes_it_from_the_rules() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let a = create(&hub, &owner, webhook("A")).await;
    let b = create(&hub, &owner, webhook("B")).await;
    let rules = vec![
        rule(vec![], vec![a, b], false),
        rule(vec![AlertKind::Down], vec![a], false),
    ];
    let response = hub
        .post_json(
            "/api/admin/alerts/routes",
            Some(&owner),
            json!({ "rules": rules }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    let response = hub
        .post_json(
            &format!("/api/admin/alerts/channels/{a}/delete"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);

    let alerts = overview(&hub).await;
    assert_eq!(alerts.rules.len(), 1, "a rule left with no channel goes");
    assert_eq!(alerts.rules[0].channels, [b]);
}

#[tokio::test]
async fn a_rule_naming_an_unknown_channel_is_refused() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let response = hub
        .post_json(
            "/api/admin/alerts/routes",
            Some(&owner),
            json!({ "rules": [rule(vec![], vec![99], false)] }),
        )
        .await;

    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.error_code(), "invalid_alerts");
}

#[tokio::test]
async fn send_test_delivers_once_and_shows_how_it_went() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let mail = json!({ "name": "Mail", "kind": "email", "enabled": true,
                       "target": "ops@example.com, oncall@example.com" });
    let id = create(&hub, &owner, mail).await;
    let send_test = || async {
        hub.post_json(
            &format!("/api/admin/alerts/channels/{id}/test"),
            Some(&owner),
            json!({}),
        )
        .await
        .json::<TestResult>()
    };

    // No mail server yet.
    let result = send_test().await;
    assert!(!result.ok);
    assert!(result.error.unwrap().contains("mail server"));

    let settings = json!({ "hub_url": "https://hub.example.com/", "smtp_server": "smtp.example.com",
        "smtp_port": 587, "smtp_security": "starttls", "smtp_username": "hub",
        "smtp_password": "mail-password", "smtp_from": "hub@example.com" });
    let response = hub
        .post_json("/api/admin/alerts/settings", Some(&owner), settings)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let alerts = overview(&hub).await;
    assert!(alerts.settings.smtp_password_set && alerts.settings.smtp_password.is_none());

    assert_eq!(
        send_test().await,
        TestResult {
            ok: true,
            error: None
        }
    );
    let Outgoing::Email {
        server,
        to,
        subject,
        body,
    } = hub.transport.sent().pop().unwrap()
    else {
        panic!("an e-mail");
    };
    assert_eq!(server.password, "mail-password", "opened for sending");
    assert_eq!(to, ["ops@example.com", "oncall@example.com"]);
    assert!(subject.ends_with("Test check TEST"), "{subject}");
    assert!(
        body.contains("https://hub.example.com/probes/test"),
        "{body}"
    );
    assert!(body.contains("This is a test"), "{body}");

    hub.transport.fail_with(Some("connection refused"));
    let result = send_test().await;
    assert_eq!(result.error.as_deref(), Some("connection refused"));
    let channel = &overview(&hub).await.channels[0];
    assert_eq!(channel.last_error.as_deref(), Some("connection refused"));
    assert!(channel.last_delivery_at.is_some());
}

/// A dispatcher over the hub's channels, with `retries` (no waits).
fn dispatcher(hub: &TestHub, tries: usize) -> Dispatcher {
    let delivery = Delivery {
        retries: vec![std::time::Duration::ZERO; tries - 1],
        ..hub_delivery(hub)
    };
    Dispatcher::new(hub.state.storage.clone(), Arc::new(delivery))
}

fn hub_delivery(hub: &TestHub) -> Delivery {
    Delivery {
        key: hub.state.delivery.key.clone(),
        transport: hub.transport.clone(),
        clock: hub.clock.clone(),
        retries: Vec::new(),
    }
}

/// Dispatch `n` and wait for every delivery.
async fn dispatch(dispatcher: &Dispatcher, n: Notification) {
    for task in dispatcher.dispatch(n).await.unwrap() {
        task.await.unwrap();
    }
}

fn alert(kind: AlertKind) -> Notification {
    Notification {
        kind,
        test: false,
        ..Notification::sample(chrono::Utc::now())
    }
}

/// Which webhooks (by URL) got deliveries, in order.
fn urls(hub: &TestHub) -> Vec<String> {
    hub.transport
        .sent()
        .into_iter()
        .map(|o| match o {
            Outgoing::Http { url, .. } => url,
            Outgoing::Email { .. } => "e-mail".into(),
        })
        .collect()
}

#[tokio::test]
async fn alerts_go_where_the_rules_say() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let to = |name: &str| {
        let mut channel = webhook(name);
        channel["target"] = json!(format!("https://example.com/{name}"));
        channel
    };
    let pager = create(&hub, &owner, to("pager")).await;
    let chat = create(&hub, &owner, to("chat")).await;
    let mut off = to("off");
    off["enabled"] = json!(false);
    let off = create(&hub, &owner, off).await;
    // Outages page and stop; everything else goes to chat (and "off").
    let rules = vec![
        rule(vec![AlertKind::Down], vec![pager], true),
        rule(vec![], vec![chat, off], false),
    ];
    let response = hub
        .post_json(
            "/api/admin/alerts/routes",
            Some(&owner),
            json!({ "rules": rules }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let dispatcher = dispatcher(&hub, 1);

    dispatch(&dispatcher, alert(AlertKind::Down)).await;
    dispatch(&dispatcher, alert(AlertKind::Recovered)).await;

    assert_eq!(
        urls(&hub),
        ["https://example.com/pager", "https://example.com/chat"]
    );
}

#[tokio::test]
async fn a_failed_delivery_is_tried_three_times_and_the_error_kept() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let id = create(&hub, &owner, webhook("Ops")).await;
    let response = hub
        .post_json(
            "/api/admin/alerts/routes",
            Some(&owner),
            json!({ "rules": [rule(vec![], vec![id], false)] }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    hub.transport.fail_with(Some("receiver answered 503"));

    dispatch(&dispatcher(&hub, 3), alert(AlertKind::Down)).await;

    assert_eq!(hub.transport.sent().len(), 3);
    let channel = &overview(&hub).await.channels[0];
    assert_eq!(channel.last_error.as_deref(), Some("receiver answered 503"));

    // The next one works, and the error is gone.
    hub.transport.fail_with(None);
    dispatch(&dispatcher(&hub, 3), alert(AlertKind::Recovered)).await;
    assert_eq!(hub.transport.sent().len(), 4);
    assert_eq!(overview(&hub).await.channels[0].last_error, None);
}

/// What a webhook receiver got: its headers and body.
#[derive(Default, Clone)]
struct Received(Arc<Mutex<Vec<(axum::http::HeaderMap, String)>>>);

#[tokio::test]
async fn a_webhook_is_signed_so_its_receiver_can_check_it() {
    let received = Received::default();
    let app = axum::Router::new()
        .route(
            "/hook",
            axum::routing::post(
                |axum::extract::State(r): axum::extract::State<Received>,
                 headers: axum::http::HeaderMap,
                 body: String| async move {
                    r.0.lock().unwrap().push((headers, body));
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .with_state(received.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let mut channel = webhook("Local");
    channel["target"] = json!(format!("http://{address}/hook"));
    let id = create(&hub, &owner, channel).await;
    let delivery = Delivery {
        transport: Arc::new(NetTransport::new().unwrap()),
        ..hub_delivery(&hub)
    };

    let result = channels::test(hub.store(), &delivery, id).await.unwrap();
    assert!(result.ok, "{:?}", result.error);

    let (headers, body) = received.0.lock().unwrap().pop().expect("a delivery");
    let timestamp = headers["x-blentinel-timestamp"].to_str().unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{body}").as_bytes());
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    assert_eq!(headers["x-blentinel-signature"].to_str().unwrap(), expected);
    assert_eq!(
        headers["content-type"].to_str().unwrap(),
        "application/json"
    );
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["event"], "test");
    assert_eq!(body["check"]["kind"], "ping");
}

#[tokio::test]
async fn an_unreachable_receiver_is_reported_without_its_url() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    // Nothing listens on port 9 (discard) of this machine.
    let mut channel = webhook("Nowhere");
    channel["target"] = json!("http://127.0.0.1:9/secret-path");
    let id = create(&hub, &owner, channel).await;
    let delivery = Delivery {
        transport: Arc::new(NetTransport::new().unwrap()),
        ..hub_delivery(&hub)
    };

    let result = channels::test(hub.store(), &delivery, id).await.unwrap();

    let error = result.error.expect("it failed");
    assert!(!error.contains("secret-path"), "{error}");
}
