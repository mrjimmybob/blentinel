//! Probe binaries served by the hub (M2-09, D-53).
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use hub_lib::model::{ProbeDownload, Role};
use sha2::{Digest, Sha256};
use support::*;

/// A hub with a Linux probe binary in `<state_dir>/probes/`.
async fn hub_with_a_probe_binary() -> TestHub {
    let hub = TestHub::new(&[]).await;
    let probes = hub.dir().join("probes");
    std::fs::create_dir_all(&probes).unwrap();
    std::fs::write(
        probes.join("probe-x86_64-unknown-linux-musl"),
        b"\x7fELF probe",
    )
    .unwrap();
    std::fs::write(probes.join("notes.txt"), b"not a probe").unwrap();
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
            sha256: hex::encode(Sha256::digest(b"\x7fELF probe")),
        }]
    );

    let viewer = hub.login_as(Role::Viewer).await;
    let refused = hub.get("/api/probe-downloads", Some(&viewer)).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_probe_downloads_without_a_session() {
    let hub = hub_with_a_probe_binary().await;
    let response = hub
        .get("/download/probe/x86_64-unknown-linux-musl", None)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(&response.body[..], b"\x7fELF probe");
    assert_eq!(
        response.headers["x-checksum-sha256"],
        hex::encode(Sha256::digest(b"\x7fELF probe")).as_str()
    );
    assert!(
        response.headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename=\"probe\"")
    );

    for missing in [
        "/download/probe/x86_64-pc-windows-msvc", // no file
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
