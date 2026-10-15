//! Companies and sites as records (M1-03, D-44): registered by reports,
//! completed and archived by admins.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use hub_lib::model::{CompanyDetail, CompanyInfo, CompanyProbe, DashboardCompany, Role};
use serde_json::json;
use support::*;

/// A hub where probe "hq" reported for company "acme" (site "hq-site").
async fn hub_with_acme() -> (TestHub, TestProbe) {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("gw", "10.0.0.1", true)])
        .await;
    (hub, probe)
}

#[tokio::test]
async fn companies_and_sites_are_created_by_hand() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let created = hub
        .post_json(
            "/api/companies",
            Some(&owner),
            json!({ "id": "acme-ltd", "name": "ACME Ltd" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text());
    let detail: CompanyDetail = created.json();
    assert_eq!(detail.company.name, "ACME Ltd");
    assert!(!detail.company.is_new);

    let taken = hub
        .post_json(
            "/api/companies",
            Some(&owner),
            json!({ "id": "acme-ltd", "name": "Again" }),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    let bad_id = hub
        .post_json(
            "/api/companies",
            Some(&owner),
            json!({ "id": "ACME Ltd", "name": "x" }),
        )
        .await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST);

    let site = hub
        .post_json(
            "/api/companies/acme-ltd/sites",
            Some(&owner),
            json!({ "id": "warehouse-1", "name": "Warehouse 1" }),
        )
        .await;
    assert_eq!(site.status, StatusCode::OK, "{}", site.text());
    let detail: CompanyDetail = site.json();
    assert_eq!(detail.sites[0].name, "Warehouse 1");
    let unknown = hub
        .post_json(
            "/api/companies/nobody/sites",
            Some(&owner),
            json!({ "id": "x", "name": "x" }),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    // Only Admins create companies; Technicians add sites to theirs.
    let tech = hub.login_as(Role::Technician).await;
    let refused = hub
        .post_json(
            "/api/companies",
            Some(&tech),
            json!({ "id": "t", "name": "T" }),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let tech_site = hub
        .post_json(
            "/api/companies/acme-ltd/sites",
            Some(&tech),
            json!({ "id": "branch", "name": "Branch" }),
        )
        .await;
    assert_eq!(tech_site.status, StatusCode::OK);
}

#[tokio::test]
async fn edits_show_on_the_dashboard_and_later_reports_keep_them() {
    let (hub, probe) = hub_with_acme().await;
    let owner = hub.login().await;

    let body = json!({ "name": "  ACME Ltd ", "contact_phone": "+34 600 000 000", "notes": "" });
    let response = hub
        .post_json("/api/companies/acme", Some(&owner), body)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let detail: CompanyDetail = response.json();
    assert_eq!(detail.company.name, "ACME Ltd", "trimmed");
    assert_eq!(detail.company.notes, None, "empty is none");
    assert!(!detail.company.is_new);

    let site = json!({ "name": "Headquarters", "address": "1 Main St" });
    let response = hub
        .post_json("/api/companies/acme/sites/hq-site", Some(&owner), site)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    hub.clock.advance(chrono::Duration::seconds(60));
    hub.report_now(&probe, "acme", vec![ping("gw", "10.0.0.1", true)])
        .await;
    let dashboard: Vec<DashboardCompany> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(dashboard[0].name, "ACME Ltd");
    let probes: Vec<CompanyProbe> = hub.get_json("/api/company/acme/probes").await;
    assert_eq!(probes[0].site.as_deref(), Some("Headquarters"));
}

#[tokio::test]
async fn an_archived_company_leaves_the_dashboard_but_still_reports() {
    let (hub, probe) = hub_with_acme().await;
    let owner = hub.login().await;

    let archive = |archived: bool| {
        hub.post_json(
            "/api/companies/acme/archive",
            Some(&owner),
            json!({ "archived": archived }),
        )
    };
    let detail: CompanyDetail = archive(true).await.json();
    assert!(detail.company.archived);
    let dashboard: Vec<DashboardCompany> = hub.get_json("/api/dashboard/companies").await;
    assert!(dashboard.is_empty());

    // Its probe keeps reporting (and alerting): archiving only hides it.
    hub.clock.advance(chrono::Duration::seconds(60));
    hub.report_now(&probe, "acme", vec![ping("gw", "10.0.0.1", false)])
        .await;
    let companies: Vec<CompanyInfo> = hub.get_json("/api/companies").await;
    assert!(companies[0].archived, "a report doesn't unarchive it");

    archive(false).await;
    let dashboard: Vec<DashboardCompany> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(dashboard.len(), 1);
}

#[tokio::test]
async fn names_are_required_and_unknown_records_are_404() {
    let (hub, _) = hub_with_acme().await;
    let owner = hub.login().await;

    let empty = hub
        .post_json("/api/companies/acme", Some(&owner), json!({ "name": "  " }))
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.error_code(), "invalid_name");

    let body = json!({ "name": "X" });
    for path in [
        "/api/companies/nobody",
        "/api/companies/nobody/sites/hq-site",
        "/api/companies/acme/sites/nowhere",
    ] {
        let response = hub.post_json(path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(
        hub.get("/api/companies/nobody", Some(&owner)).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn viewers_read_companies_but_cannot_edit_them() {
    let (hub, _) = hub_with_acme().await;
    let viewer = hub.login_as(Role::Viewer).await;

    assert_eq!(
        hub.get("/api/companies/acme", Some(&viewer)).await.status,
        StatusCode::OK
    );
    let response = hub
        .post_json(
            "/api/companies/acme",
            Some(&viewer),
            json!({ "name": "Mine" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}
