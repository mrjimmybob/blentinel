//! Probe binaries served by the hub for "Add probe" (M2-09, D-53), from its
//! signed releases (D-55).
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use hub_lib::model::{ProbeDownload, Role};
use sha2::{Digest, Sha256};
use support::*;

/// A hub with a release holding a Linux probe.
async fn hub_with_a_probe_binary() -> TestHub {
    let hub = TestHub::new(&[]).await;
    hub.add_release("0.1.0", &[("x86_64-unknown-linux-musl", b"ELF probe")]);
    hub
}

#[tokio::test]
async fn the_wizard_lists_the_probes_the_hub_has() {
    let hub = hub_with_a_probe_binary().await;
    let downloads: Vec<ProbeDownload> = hub.get_json("/api/probe-downloads").await;
    assert_eq!(
        downloads,
        [ProbeDownload {
            target: "x86_64-unknown-linux-musl".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            size: 10,
            sha256: hex::encode(Sha256::digest(b"ELF probe")),
            version: "0.1.0".into(),
        }]
    );

    let viewer = hub.login_as(Role::Viewer).await;
    let refused = hub.get("/api/probe-downloads", Some(&viewer)).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn without_a_release_there_is_nothing_to_download() {
    let hub = TestHub::new(&[]).await;
    let downloads: Vec<ProbeDownload> = hub.get_json("/api/probe-downloads").await;
    assert!(downloads.is_empty());
}

#[tokio::test]
async fn add_probe_offers_the_approved_release_else_the_newest() {
    let hub = hub_with_a_probe_binary().await;
    hub.add_release("0.2.0", &[("x86_64-unknown-linux-musl", b"probe two")]);
    let version = |downloads: Vec<ProbeDownload>| downloads[0].version.clone();
    assert_eq!(version(hub.get_json("/api/probe-downloads").await), "0.2.0");

    let owner = hub.login().await;
    let approved = hub
        .post_json(
            "/api/admin/probe-updates/approve",
            Some(&owner),
            serde_json::json!({ "version": "0.1.0" }),
        )
        .await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.text());
    assert_eq!(version(hub.get_json("/api/probe-downloads").await), "0.1.0");
    let response = hub
        .get("/download/probe/x86_64-unknown-linux-musl", None)
        .await;
    assert_eq!(&response.body[..], b"ELF probe");
}

#[tokio::test]
async fn a_probe_downloads_without_a_session() {
    let hub = hub_with_a_probe_binary().await;
    let response = hub
        .get("/download/probe/x86_64-unknown-linux-musl", None)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(&response.body[..], b"ELF probe");
    assert_eq!(
        response.headers["x-checksum-sha256"],
        hex::encode(Sha256::digest(b"ELF probe")).as_str()
    );
    assert!(
        response.headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename=\"probe\"")
    );

    for missing in [
        "/download/probe/x86_64-pc-windows-msvc", // not in the release
        "/download/probe/..%2Fblentinel.db",      // not a target
        "/download/probe/notes.txt",
    ] {
        let response = hub.get(missing, None).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{missing}");
    }
}

#[tokio::test]
async fn a_new_code_says_where_probes_reach_the_hub() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    hub.post_json(
        "/api/companies",
        Some(&owner),
        serde_json::json!({ "id": "acme", "name": "ACME" }),
    )
    .await;
    hub.post_json(
        "/api/companies/acme/sites",
        Some(&owner),
        serde_json::json!({ "id": "hq", "name": "HQ" }),
    )
    .await;
    let created = hub
        .post_json(
            "/api/enrollment-codes",
            Some(&owner),
            serde_json::json!({ "company_id": "acme", "site_id": "hq", "hub_url": "http://hub.lan:3000/" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text());
    let created: hub_lib::model::CreatedCode = created.json();
    assert_eq!(created.hub_url, "http://hub.lan:3000");
}
