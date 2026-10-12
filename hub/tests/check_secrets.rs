//! Secrets on a check (M5-08, D-86), through the API: sealed in the
//! database, never shown, sent to the probe with its checks.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::ProbeConfig;
use hub_lib::model::{AddedChecks, CheckInfo, ProbeChecks, TemplateInfo};
use serde_json::{Value, json};
use sqlx::Row;
use support::*;

const COMMUNITY: &str = "s3cret-community";

fn switch(extra: Value) -> Value {
    let mut check = json!({
        "name": "Switch",
        "spec": { "kind": "snmp", "host": "192.168.1.5", "oids": [{ "oid": "1.3.6.1.2.1.1.3.0", "name": "Uptime" }] },
    });
    for (key, value) in extra.as_object().unwrap() {
        check[key] = value.clone();
    }
    check
}

async fn post(hub: &TestHub, cookie: &str, path: &str, body: Value) -> TestResponse {
    hub.post_json(path, Some(cookie), body).await
}

async fn add(hub: &TestHub, cookie: &str, probe: &TestProbe, body: Value) -> CheckInfo {
    let response = post(
        hub,
        cookie,
        &format!("/api/probes/{}/checks", probe.id),
        body,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

async fn checks_of(hub: &TestHub, probe: &TestProbe) -> ProbeChecks {
    hub.get_json(&format!("/api/probes/{}/checks", probe.id))
        .await
}

/// Report from `probe` a minute later, running `revision`; the config in
/// the hub's answer, if it sent one.
async fn config_for(
    hub: &TestHub,
    probe: &TestProbe,
    revision: Option<&str>,
) -> Option<ProbeConfig> {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.config_revision = revision.map(str::to_string);
    probe.sign(&mut report);
    let (status, ack) = hub.send_report_for_ack(probe, &report).await;
    assert_eq!(status, StatusCode::OK);
    ack.expect("a trusted ack").config
}

/// The secret goes in, is sealed in the database, reaches the probe in the
/// hub's answer, and is in nothing a person can read.
#[tokio::test]
async fn a_checks_secret_is_sealed_never_shown_and_sent_to_the_probe() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let body = switch(json!({ "secrets": { "community": COMMUNITY } }));
    let response = post(
        &hub,
        &owner,
        &format!("/api/probes/{}/checks", probe.id),
        body,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert!(!response.text().contains(COMMUNITY), "not in the answer");
    let check: CheckInfo = response.json();
    assert_eq!(check.secrets_set, ["community"]);

    let listed = hub
        .get(&format!("/api/probes/{}/checks", probe.id), Some(&owner))
        .await;
    assert!(!listed.text().contains(COMMUNITY), "not in the list");
    let audit = hub
        .get("/api/audit?action=check.create", Some(&owner))
        .await;
    assert!(audit.text().contains("community"), "the audit log names it");
    assert!(!audit.text().contains(COMMUNITY), "and doesn't have it");

    let row = sqlx::query("SELECT spec, secrets FROM checks WHERE id = ?")
        .bind(&check.id)
        .fetch_one(hub.pool())
        .await
        .unwrap();
    let (spec, stored): (String, String) = (row.get("spec"), row.get("secrets"));
    assert!(
        !spec.contains(COMMUNITY) && !stored.contains(COMMUNITY),
        "{stored}"
    );
    assert!(
        stored.contains("\"community\""),
        "by name, sealed: {stored}"
    );

    // The probe gets it with the check, in the sealed answer.
    let config = config_for(&hub, &probe, None)
        .await
        .expect("a first config");
    assert_eq!(config.checks.len(), 1);
    assert_eq!(config.checks[0].secrets.get("community"), Some(COMMUNITY));
    assert_eq!(
        config.validate(),
        Ok(()),
        "what the probe checks before using it"
    );
    assert_eq!(
        config.revision,
        checks_of(&hub, &probe).await.config_revision
    );
    // And nothing more once it runs it.
    assert!(
        config_for(&hub, &probe, Some(&config.revision))
            .await
            .is_none()
    );
}

/// Editing a check keeps its secret unless a new one is typed; a new one
/// reaches the probe as a change.
#[tokio::test]
async fn an_edit_keeps_the_secret_unless_a_new_one_is_given() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let check = add(
        &hub,
        &owner,
        &probe,
        switch(json!({ "secrets": { "community": COMMUNITY } })),
    )
    .await;
    let path = format!("/api/checks/{}", check.id);
    let first = config_for(&hub, &probe, None).await.unwrap().revision;

    // The same check again, nothing typed: nothing changes for the probe.
    for body in [
        switch(json!({})),
        switch(json!({ "secrets": { "community": "" } })),
    ] {
        let response = post(&hub, &owner, &path, body).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        assert_eq!(response.json::<CheckInfo>().secrets_set, ["community"]);
    }
    assert!(
        config_for(&hub, &probe, Some(&first)).await.is_none(),
        "the same revision"
    );

    // A new community: a new revision, and the probe gets the new one.
    let response = post(
        &hub,
        &owner,
        &path,
        switch(json!({ "secrets": { "community": "another" } })),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let config = config_for(&hub, &probe, Some(&first))
        .await
        .expect("a changed config");
    assert_ne!(config.revision, first);
    assert_eq!(config.checks[0].secrets.get("community"), Some("another"));

    // Typing the same one again is a change too, and says nothing about it.
    post(
        &hub,
        &owner,
        &path,
        switch(json!({ "secrets": { "community": "another" } })),
    )
    .await;
    assert_ne!(
        checks_of(&hub, &probe).await.config_revision,
        config.revision
    );
}

/// A check that runs must have what it needs; a disabled one may wait.
#[tokio::test]
async fn a_check_that_runs_needs_its_secrets_and_takes_no_others() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let create = format!("/api/probes/{}/checks", probe.id);

    let missing = post(&hub, &owner, &create, switch(json!({}))).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert!(
        missing.text().contains("the community is needed"),
        "{}",
        missing.text()
    );
    let unknown = post(
        &hub,
        &owner,
        &create,
        switch(json!({ "secrets": { "password": "x" } })),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert!(
        unknown.text().contains("no secret named password"),
        "{}",
        unknown.text()
    );
    let cpu = json!({ "name": "CPU", "spec": { "kind": "cpu" }, "secrets": { "community": "x" } });
    assert_eq!(
        post(&hub, &owner, &create, cpu).await.status,
        StatusCode::BAD_REQUEST
    );

    // Disabled, it waits for its community.
    let waiting = add(&hub, &owner, &probe, switch(json!({ "enabled": false }))).await;
    assert!(waiting.secrets_set.is_empty());
    let path = format!("/api/checks/{}", waiting.id);
    let enable = post(&hub, &owner, &path, switch(json!({}))).await;
    assert_eq!(enable.status, StatusCode::BAD_REQUEST, "not without it");
    let enable = post(
        &hub,
        &owner,
        &path,
        switch(json!({ "secrets": { "community": COMMUNITY } })),
    )
    .await;
    assert_eq!(enable.status, StatusCode::OK, "{}", enable.text());

    // SNMP v3: the community goes, the passwords come, and they have a length.
    let v3 = |secrets: Value| {
        json!({
            "name": "Switch",
            "spec": {
                "kind": "snmp", "host": "192.168.1.5", "version": "v3", "user": "monitor",
                "auth": "sha256", "privacy": "aes128",
                "oids": [{ "oid": "1.3.6.1.2.1.1.3.0" }],
            },
            "secrets": secrets,
        })
    };
    let short = post(
        &hub,
        &owner,
        &path,
        v3(json!({ "auth_password": "short", "priv_password": "long enough" })),
    )
    .await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST);
    assert!(
        short.text().contains("at least 8 characters"),
        "{}",
        short.text()
    );
    let half = post(
        &hub,
        &owner,
        &path,
        v3(json!({ "auth_password": "long enough" })),
    )
    .await;
    assert!(
        half.text().contains("the privacy password is needed"),
        "{}",
        half.text()
    );
    let good = post(
        &hub,
        &owner,
        &path,
        v3(json!({ "auth_password": "long enough", "priv_password": "long enough too" })),
    )
    .await;
    assert_eq!(good.status, StatusCode::OK, "{}", good.text());
    assert_eq!(
        good.json::<CheckInfo>().secrets_set,
        ["auth_password", "priv_password"],
        "the community is gone with v2c"
    );
    let config = config_for(&hub, &probe, None).await.unwrap();
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(
        config.checks[0].secrets.names(),
        ["auth_password", "priv_password"]
    );
}

/// Copying a probe's checks takes their secrets along; a template keeps
/// none, so a check made from one waits, disabled, for its own.
#[tokio::test]
async fn copies_keep_secrets_and_templates_dont() {
    let (first, second, third) = (
        TestProbe::new("first"),
        TestProbe::new("second"),
        TestProbe::new("third"),
    );
    let hub = TestHub::new(&[&first, &second, &third]).await;
    let owner = hub.login().await;
    add(
        &hub,
        &owner,
        &first,
        switch(json!({ "secrets": { "community": COMMUNITY } })),
    )
    .await;

    let copy = post(
        &hub,
        &owner,
        &format!("/api/probes/{}/checks/copy", second.id),
        json!({ "from_probe_id": first.id }),
    )
    .await;
    assert_eq!(copy.status, StatusCode::OK, "{}", copy.text());
    assert!(!copy.text().contains(COMMUNITY));
    let config = config_for(&hub, &second, None).await.unwrap();
    assert_eq!(config.checks[0].secrets.get("community"), Some(COMMUNITY));

    let saved = post(
        &hub,
        &owner,
        "/api/check-templates",
        json!({ "name": "Switches", "from_probe_id": first.id }),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert!(
        !saved.text().contains(COMMUNITY),
        "a template has no secrets"
    );
    let template: TemplateInfo = saved.json();
    let applied = post(
        &hub,
        &owner,
        &format!("/api/probes/{}/checks/template", third.id),
        json!({ "template_id": template.id }),
    )
    .await;
    assert_eq!(applied.status, StatusCode::OK, "{}", applied.text());
    let added: AddedChecks = applied.json();
    assert_eq!(added.added.len(), 1);
    assert!(!added.added[0].enabled, "it waits for its community");
    assert!(added.added[0].secrets_set.is_empty());
    // The probe isn't sent a check it couldn't run.
    let config = config_for(&hub, &third, None).await.unwrap();
    assert!(config.checks.is_empty());
    assert_eq!(config.validate(), Ok(()));
}
