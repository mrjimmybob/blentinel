//! Config hot reload (M0-23, D-38): every setting applies without a restart,
//! and a reload that can't be applied changes nothing.
#![cfg(feature = "ssr")]

mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use support::*;

const DB_LINE: &str = "db_path = 'blentinel.db'\n";

fn ping_up() -> Vec<common::models::ResourceStatus> {
    vec![ping("gw", "10.0.0.1", true)]
}

async fn login_status(hub: &TestHub, token: &str) -> StatusCode {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/login")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(token.to_string()))
        .unwrap();
    hub.send(request).await.status
}

async fn company_count(hub: &TestHub) -> usize {
    hub.get_json::<Vec<serde_json::Value>>("/api/dashboard/companies")
        .await
        .len()
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_invalid_file_changes_nothing() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let empty_host = hub.edited_config("host = '127.0.0.1'", "host = ''");

    assert!(hub.reload_with("this is [not toml").await.is_err());
    assert!(
        hub.reload_with(&empty_host).await.is_err(),
        "fails validation"
    );
    assert_eq!(hub.state.config.read().await.server.host, "127.0.0.1");

    // Still the old config: the probe is still whitelisted.
    let report = probe.signed_report("acme", hub.now(), ping_up());
    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

#[tokio::test]
async fn a_newly_whitelisted_probe_is_accepted() {
    let hub = TestHub::new(&[]).await;
    let probe = TestProbe::new("new");
    let report = probe.signed_report("acme", hub.now(), ping_up());
    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);

    let mut toml = std::fs::read_to_string(hub.config_path()).unwrap();
    toml.push_str(&format!(
        "\n[[probes]]\nname = 'new'\npublic_key = '{}'\n",
        probe.id
    ));
    hub.reload_with(&toml).await.unwrap();

    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_db_path_switches_databases() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", ping_up()).await;
    assert_eq!(company_count(&hub).await, 1);

    let other = hub.edited_config(DB_LINE, "db_path = 'other.db'\n");
    hub.reload_with(&other).await.unwrap();
    assert!(hub.dir().join("other.db").exists(), "created on reload");
    assert_eq!(company_count(&hub).await, 0, "the new database is empty");

    // Reports now go to the new database.
    hub.clock.advance(chrono::Duration::seconds(60));
    hub.report_now(&probe, "beta", ping_up()).await;
    let companies: Vec<serde_json::Value> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(companies[0]["company_id"], "beta");

    // And back: the first database still has its data.
    let original = hub.edited_config("db_path = 'other.db'\n", DB_LINE);
    hub.reload_with(&original).await.unwrap();
    let companies: Vec<serde_json::Value> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(companies.len(), 1);
    assert_eq!(companies[0]["company_id"], "acme");
}

#[tokio::test]
async fn a_database_that_cannot_be_opened_changes_nothing() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", ping_up()).await;
    // A directory where the database file should be.
    std::fs::create_dir(hub.dir().join("not-a-file.db")).unwrap();

    let broken = hub.edited_config(DB_LINE, "db_path = 'not-a-file.db'\n");
    let error = hub.reload_with(&broken).await.unwrap_err();
    assert!(format!("{error:#}").contains("not-a-file.db"), "{error:#}");

    assert_eq!(company_count(&hub).await, 1, "still the old database");
}

// ---------------------------------------------------------------------------
// Hub key and admin token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_identity_key_path_changes_the_hub_key() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let old_key = hub.public_key();

    let new_path = hub.edited_config(
        DB_LINE,
        &format!("{DB_LINE}identity_key_path = 'rotated.key'\n"),
    );
    hub.reload_with(&new_path).await.unwrap();

    let new_key = hub.public_key();
    assert_ne!(old_key, new_key);
    assert!(hub.dir().join("rotated.key").exists());
    let handshake = hub.get("/api/handshake", None).await;
    assert_eq!(handshake.text(), hex::encode(new_key));

    // Sealed for the old key: can't be opened any more. For the new one: accepted.
    let report = probe.signed_report("acme", hub.now(), ping_up());
    let stale = hub.post_bytes("/api/report", seal(&report, &old_key)).await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED);
    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

#[tokio::test]
async fn a_reset_token_file_applies_at_the_next_login() {
    let hub = TestHub::new(&[]).await;
    assert_eq!(login_status(&hub, ADMIN_TOKEN).await, StatusCode::OK);

    // What `hub --reset-admin-token` does to a running hub's token file.
    std::fs::write(hub.dir().join("hub_auth.token"), "brand-new-token").unwrap();

    assert_eq!(
        login_status(&hub, ADMIN_TOKEN).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(login_status(&hub, "brand-new-token").await, StatusCode::OK);
}

#[tokio::test]
async fn a_new_auth_token_path_is_created_and_used() {
    let hub = TestHub::new(&[]).await;

    let moved = hub.edited_config(
        DB_LINE,
        &format!("{DB_LINE}auth_token_path = 'moved.token'\n"),
    );
    hub.reload_with(&moved).await.unwrap();

    let token = std::fs::read_to_string(hub.dir().join("moved.token")).unwrap();
    assert_ne!(token.trim(), ADMIN_TOKEN);
    assert_eq!(login_status(&hub, token.trim()).await, StatusCode::OK);
    assert_eq!(
        login_status(&hub, ADMIN_TOKEN).await,
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------
// Listening address (the switch itself is tested in listeners.rs)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_port_in_use_refuses_the_reload() {
    let hub = TestHub::new(&[]).await;
    let busy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = busy.local_addr().unwrap().port();

    let taken = hub.edited_config("port = 3000", &format!("port = {port}"));
    let error = hub.reload_with(&taken).await.unwrap_err();
    assert!(format!("{error:#}").contains("can't listen"), "{error:#}");
    assert_eq!(hub.state.config.read().await.server.port, 3000);
}

#[tokio::test]
async fn a_new_port_wakes_the_listeners() {
    let hub = TestHub::new(&[]).await;
    let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);

    let woken = hub.state.listeners_changed.notified();
    let moved = hub.edited_config("port = 3000", &format!("port = {port}"));
    hub.reload_with(&moved).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), woken)
        .await
        .expect("listeners notified");
    assert_eq!(hub.state.config.read().await.server.port, port);
}
