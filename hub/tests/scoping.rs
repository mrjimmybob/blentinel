//! Per-user company scope (M1-04, D-45): every route that returns or changes
//! company data answers only within the user's scope. Outside it: absent
//! from lists, 404 for single items (never 403, so other customers can't be
//! discovered).
#![cfg(feature = "ssr")]

mod support;

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use hub_lib::auth::Access;
use hub_lib::model::{
    AlertSilence, CompanyDetail, CompanyInfo, CompanyScope, DashboardCompany, Permission, Role,
};
use hub_lib::routes::access_table;
use hub_lib::service::users::{self, UserFields};
use serde_json::{Value, json};
use support::*;

/// Every route that returns or changes company data, each exercised below.
/// A new data route must be added here, with a test, or to `NOT_COMPANY_DATA`.
const COVERED: &[(&str, &str)] = &[
    ("GET", "/api/companies"),
    ("GET", "/api/companies/{id}"),
    ("POST", "/api/companies/{id}"),
    ("POST", "/api/companies/{id}/sites/{site}"),
    ("POST", "/api/companies/{id}/sites"),
    ("GET", "/api/probes"),
    ("POST", "/api/probes/{id}"),
    ("GET", "/api/enrollment-codes"),
    ("POST", "/api/enrollment-codes"),
    ("POST", "/api/enrollment-codes/{id}/revoke"),
    ("GET", "/api/dashboard/companies"),
    ("GET", "/api/company/{company_id}/probes"),
    ("GET", "/api/company/{company_id}/uptime"),
    ("GET", "/api/probe/{probe_id}/devices"),
    ("GET", "/api/silences"),
    ("POST", "/api/silence"),
    ("POST", "/api/silence/delete"),
    ("GET", "/api/archive/{archive_id}/companies"),
    (
        "GET",
        "/api/archive/{archive_id}/company/{company_id}/probes",
    ),
    (
        "GET",
        "/api/archive/{archive_id}/company/{company_id}/uptime",
    ),
    ("GET", "/api/archive/{archive_id}/probe/{probe_id}/devices"),
];

/// Routes open to scoped users that carry no company data.
const NOT_COMPANY_DATA: &[(&str, &str)] = &[("POST", "/api/me/logout-others")];

#[test]
fn every_route_open_to_scoped_users_is_covered() {
    // Routes a non-global role can reach, other than its own account.
    let scoped_routes: BTreeSet<(String, &str)> = access_table()
        .into_iter()
        .filter(|r| match r.access {
            Access::Perm(p) => [Role::Technician, Role::Viewer, Role::Client]
                .into_iter()
                .any(|role| hub_lib::model::authorize(role, p)),
            _ => false,
        })
        .map(|r| (r.method.to_string(), r.path))
        .collect();
    let decided: BTreeSet<(String, &str)> = COVERED
        .iter()
        .chain(NOT_COMPANY_DATA)
        .map(|(m, p)| (m.to_string(), *p))
        .collect();
    assert_eq!(
        scoped_routes, decided,
        "a route open to scoped users needs a scoping decision (COVERED + a test, or NOT_COMPANY_DATA)"
    );
    assert!(hub_lib::model::authorize(
        Role::Technician,
        Permission::ManageCompanies
    ));
}

// ---------------------------------------------------------------------------
// Setup: three companies, users with different scopes
// ---------------------------------------------------------------------------

fn ids(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

struct World {
    hub: TestHub,
    /// Probe IDs by company.
    probe: std::collections::HashMap<&'static str, String>,
}

/// Companies acme, beta and zeta, one probe each, a silence in each.
async fn world() -> World {
    let probes = [
        ("acme", TestProbe::new("pa")),
        ("beta", TestProbe::new("pb")),
        ("zeta", TestProbe::new("pz")),
    ];
    let refs: Vec<&TestProbe> = probes.iter().map(|(_, p)| p).collect();
    let hub = TestHub::new(&refs).await;
    let owner = hub.login().await;
    for (company, probe) in &probes {
        hub.report_now(probe, company, vec![ping("gw", "10.0.0.1", true)])
            .await;
        let key = hub_lib::model::resource_key(company, &probe.id, "gw", "10.0.0.1");
        let body = json!({ "resource_key": key, "reason": "test", "duration_hours": 1 });
        hub.post_json("/api/silence", Some(&owner), body).await;
    }
    World {
        probe: probes.iter().map(|(c, p)| (*c, p.id.clone())).collect(),
        hub,
    }
}

impl World {
    /// Log in as a new user with `role` and `scope`.
    async fn as_user(&self, username: &str, role: Role, scope: CompanyScope) -> String {
        let fields = UserFields {
            display_name: String::new(),
            email: None,
            role,
            scope,
        };
        let store = self.hub.state.storage.get();
        users::create_user(
            store.as_ref(),
            None,
            username,
            fields,
            Some(PASSWORD),
            self.hub.now(),
        )
        .await
        .expect("create scoped user");
        self.hub.login_user(username).await
    }

    async fn tech_acme(&self) -> String {
        let scope = CompanyScope::Only(ids(&["acme"]));
        self.as_user("tech", Role::Technician, scope).await
    }

    async fn boss_all_but_zeta(&self) -> String {
        let scope = CompanyScope::AllExcept(ids(&["zeta"]));
        self.as_user("boss", Role::Viewer, scope).await
    }

    async fn client_acme_beta(&self) -> String {
        let scope = CompanyScope::Only(ids(&["acme", "beta"]));
        self.as_user("client", Role::Client, scope).await
    }

    async fn get(&self, path: &str, cookie: &str) -> TestResponse {
        self.hub.get(path, Some(cookie)).await
    }
}

fn company_ids<T>(list: &[T], id: impl Fn(&T) -> &str) -> Vec<String> {
    list.iter().map(|c| id(c).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lists_show_only_companies_in_scope() {
    let w = world().await;
    let cases = [
        (w.tech_acme().await, vec!["acme"]),
        (w.boss_all_but_zeta().await, vec!["acme", "beta"]),
        (w.client_acme_beta().await, vec!["acme", "beta"]),
        (w.hub.login().await, vec!["acme", "beta", "zeta"]),
    ];
    for (cookie, expected) in cases {
        let dashboard: Vec<DashboardCompany> =
            w.get("/api/dashboard/companies", &cookie).await.json();
        assert_eq!(company_ids(&dashboard, |c| &c.company_id), expected);
        let companies: Vec<CompanyInfo> = w.get("/api/companies", &cookie).await.json();
        assert_eq!(company_ids(&companies, |c| &c.id), expected);
        let silences: Vec<AlertSilence> = w.get("/api/silences", &cookie).await.json();
        let mut silenced: Vec<String> = silences
            .iter()
            .map(|s| hub_lib::model::company_of_resource_key(&s.scope_id).to_string())
            .collect();
        silenced.sort();
        assert_eq!(silenced, expected);
    }
}

#[tokio::test]
async fn all_except_includes_companies_registered_later() {
    let w = world().await;
    let boss = w.boss_all_but_zeta().await;

    // A new customer: company, site and probe added after the boss's user.
    let newcomer = TestProbe::new("pn");
    w.hub
        .place_probe(&newcomer.id, "pn", "newco", "pn-site")
        .await;
    w.hub
        .report_now(&newcomer, "newco", vec![ping("gw", "10.0.0.9", true)])
        .await;

    let dashboard: Vec<DashboardCompany> = w.get("/api/dashboard/companies", &boss).await.json();
    assert!(dashboard.iter().any(|c| c.company_id == "newco"));
}

// ---------------------------------------------------------------------------
// Single items: in scope works, out of scope is 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn single_items_outside_the_scope_are_not_found() {
    let w = world().await;
    let tech = w.tech_acme().await;
    let (acme_probe, zeta_probe) = (&w.probe["acme"], &w.probe["zeta"]);

    for (path, in_scope) in [
        ("/api/companies/acme".to_string(), true),
        ("/api/companies/zeta".to_string(), false),
        ("/api/company/acme/probes".to_string(), true),
        ("/api/company/zeta/probes".to_string(), false),
        ("/api/company/acme/uptime?range=7d".to_string(), true),
        ("/api/company/zeta/uptime".to_string(), false),
        (format!("/api/probe/{acme_probe}/devices"), true),
        (format!("/api/probe/{zeta_probe}/devices"), false),
        ("/api/probe/unknown/devices".to_string(), false),
    ] {
        let response = w.get(&path, &tech).await;
        let expected = if in_scope {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        };
        assert_eq!(response.status, expected, "{path}");
        if !in_scope {
            assert_eq!(response.error_code(), "not_found", "{path}");
        }
    }
}

#[tokio::test]
async fn technicians_edit_and_silence_only_their_companies() {
    let w = world().await;
    let tech = w.tech_acme().await;
    let post = |path: &'static str, body: Value| w.hub.post_json(path, Some(&tech), body);

    let rename = json!({ "name": "ACME Ltd", "notes": "gate code 1234" });
    assert_eq!(
        post("/api/companies/acme", rename.clone()).await.status,
        StatusCode::OK
    );
    assert_eq!(
        post("/api/companies/zeta", rename).await.status,
        StatusCode::NOT_FOUND
    );
    let site = json!({ "name": "HQ" });
    assert_eq!(
        post("/api/companies/acme/sites/pa-site", site.clone())
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        post("/api/companies/zeta/sites/pz-site", site).await.status,
        StatusCode::NOT_FOUND
    );
    // Archiving hides a company from everyone: Admins only.
    let archive = post("/api/companies/acme/archive", json!({ "archived": true })).await;
    assert_eq!(archive.status, StatusCode::FORBIDDEN);

    // Silences: create and delete only in scope.
    let key = |company: &str| hub_lib::model::resource_key(company, &w.probe[company], "gw", "x");
    let silence =
        |company: &str| json!({ "resource_key": key(company), "reason": "r", "duration_hours": 1 });
    let own = w
        .hub
        .post_json("/api/silence", Some(&tech), silence("acme"))
        .await;
    assert_eq!(own.status, StatusCode::OK);
    let other = w
        .hub
        .post_json("/api/silence", Some(&tech), silence("zeta"))
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);

    let owner = w.hub.login().await;
    let all: Vec<AlertSilence> = w.get("/api/silences", &owner).await.json();
    let zeta_silence = all
        .iter()
        .find(|s| s.scope_id.starts_with("zeta:"))
        .unwrap()
        .id;
    let delete = |id: i64| {
        w.hub.post_json(
            "/api/silence/delete",
            Some(&tech),
            json!({ "silence_id": id }),
        )
    };
    assert_eq!(delete(zeta_silence).await.status, StatusCode::NOT_FOUND);
    let still: Vec<AlertSilence> = w.get("/api/silences", &owner).await.json();
    assert!(still.iter().any(|s| s.id == zeta_silence), "not deleted");
    let acme_silence = still
        .iter()
        .find(|s| s.scope_id.starts_with("acme:"))
        .unwrap()
        .id;
    assert_eq!(delete(acme_silence).await.status, StatusCode::OK);
}

#[tokio::test]
async fn clients_see_their_companies_without_internal_notes_and_are_read_only() {
    let w = world().await;
    let owner = w.hub.login().await;
    let internal = json!({
        "name": "ACME", "notes": "internal", "contact_name": "Ann",
        "contact_email": "ann@acme.example", "contact_phone": "1",
    });
    w.hub
        .post_json("/api/companies/acme", Some(&owner), internal)
        .await;
    let site = json!({ "name": "HQ", "address": "1 Main St", "notes": "alarm code" });
    w.hub
        .post_json("/api/companies/acme/sites/pa-site", Some(&owner), site)
        .await;
    let client = w.client_acme_beta().await;

    let detail: CompanyDetail = w.get("/api/companies/acme", &client).await.json();
    assert_eq!(detail.company.name, "ACME");
    assert_eq!(detail.company.notes, None);
    assert_eq!(detail.company.contact_name, None);
    assert_eq!(detail.company.contact_email, None);
    assert_eq!(
        detail.sites[0].address.as_deref(),
        Some("1 Main St"),
        "addresses stay"
    );
    assert_eq!(detail.sites[0].notes, None);
    let list: Vec<CompanyInfo> = w.get("/api/companies", &client).await.json();
    assert!(
        list.iter()
            .all(|c| c.notes.is_none() && c.contact_email.is_none())
    );

    // Device detail of their own companies: yes. Other companies: no.
    let devices = format!("/api/probe/{}/devices", w.probe["acme"]);
    assert_eq!(w.get(&devices, &client).await.status, StatusCode::OK);
    assert_eq!(
        w.get("/api/companies/zeta", &client).await.status,
        StatusCode::NOT_FOUND
    );
    // Read-only.
    let key = hub_lib::model::resource_key("acme", &w.probe["acme"], "gw", "x");
    let silence = json!({ "resource_key": key, "reason": "r", "duration_hours": 1 });
    let response = w
        .hub
        .post_json("/api/silence", Some(&client), silence)
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let edit = w
        .hub
        .post_json(
            "/api/companies/acme",
            Some(&client),
            json!({ "name": "Mine" }),
        )
        .await;
    assert_eq!(edit.status, StatusCode::FORBIDDEN);
    // Internal staff still see the notes.
    let tech = w.tech_acme().await;
    let detail: CompanyDetail = w.get("/api/companies/acme", &tech).await.json();
    assert_eq!(detail.company.notes.as_deref(), Some("internal"));
}

// ---------------------------------------------------------------------------
// Archives follow the same scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn archives_are_scoped_too() {
    // The archive engine reads the real clock: start at the real "now".
    let (pa, pz) = (TestProbe::new("pa"), TestProbe::new("pz"));
    let hub = TestHub::starting_at(Utc::now(), &[&pa, &pz]).await;
    for (company, probe) in [("acme", &pa), ("zeta", &pz)] {
        let old = probe.signed_report(
            company,
            hub.now() - Duration::days(100),
            vec![ping("gw", "1", true)],
        );
        hub.seed_report(&old).await;
        hub.report_now(probe, company, vec![ping("gw", "1", true)])
            .await;
    }
    let owner = hub.login().await;
    hub.post_json(
        "/api/admin/archive",
        Some(&owner),
        json!({ "cutoff_days": 90 }),
    )
    .await;
    let archives: Vec<Value> = hub.get_json("/api/admin/archives").await;
    let id = archives[0]["id"].as_i64().unwrap();

    let fields = UserFields {
        display_name: String::new(),
        email: None,
        role: Role::Viewer,
        scope: CompanyScope::Only(ids(&["acme"])),
    };
    let store = hub.state.storage.get();
    users::create_user(store.as_ref(), None, "v", fields, Some(PASSWORD), hub.now())
        .await
        .unwrap();
    let viewer = hub.login_user("v").await;

    let companies: Vec<DashboardCompany> = hub
        .get(&format!("/api/archive/{id}/companies"), Some(&viewer))
        .await
        .json();
    assert_eq!(company_ids(&companies, |c| &c.company_id), ["acme"]);
    for (path, expected) in [
        (
            format!("/api/archive/{id}/company/acme/probes"),
            StatusCode::OK,
        ),
        (
            format!("/api/archive/{id}/company/zeta/probes"),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/api/archive/{id}/company/acme/uptime"),
            StatusCode::OK,
        ),
        (
            format!("/api/archive/{id}/company/zeta/uptime"),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/api/archive/{id}/probe/{}/devices", pa.id),
            StatusCode::OK,
        ),
        (
            format!("/api/archive/{id}/probe/{}/devices", pz.id),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(
            hub.get(&path, Some(&viewer)).await.status,
            expected,
            "{path}"
        );
    }
    let _ = Method::GET; // (used by the guard via access_table)
}

// ---------------------------------------------------------------------------
// Probes, enrollment codes and new sites (M2-01, D-47)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn probes_codes_and_sites_follow_the_scope() {
    let w = world().await;
    let owner = w.hub.login().await;
    let tech = w.tech_acme().await;
    let code = |company: &str| {
        json!({
            "company_id": company, "site_id": format!("p{}-site", &company[..1]),
            "hub_url": "https://hub.example.com:3443",
        })
    };

    // Probe records.
    let probes: Vec<hub_lib::model::ProbeInfo> = w.get("/api/probes", &tech).await.json();
    let companies: Vec<_> = probes.iter().map(|p| p.company_id.as_str()).collect();
    assert_eq!(companies, ["acme"]);
    let rename = async |id: &str| {
        let body = json!({ "name": "renamed", "site_id": "pa-site" });
        let path = format!("/api/probes/{id}");
        w.hub.post_json(&path, Some(&tech), body).await.status
    };
    assert_eq!(rename(&w.probe["acme"]).await, StatusCode::OK);
    assert_eq!(rename(&w.probe["zeta"]).await, StatusCode::NOT_FOUND);

    // Enrollment codes: create, list, revoke.
    let zeta_code: hub_lib::model::CreatedCode = w
        .hub
        .post_json("/api/enrollment-codes", Some(&owner), code("zeta"))
        .await
        .json();
    let own = w
        .hub
        .post_json("/api/enrollment-codes", Some(&tech), code("acme"))
        .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text());
    let other = w
        .hub
        .post_json("/api/enrollment-codes", Some(&tech), code("zeta"))
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    let codes: Vec<hub_lib::model::EnrollmentCodeInfo> =
        w.get("/api/enrollment-codes", &tech).await.json();
    assert!(codes.iter().all(|c| c.company_id == "acme"));
    assert_eq!(codes.len(), 1);
    let revoke = format!("/api/enrollment-codes/{}/revoke", zeta_code.info.id);
    let refused = w.hub.post_json(&revoke, Some(&tech), json!({})).await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);

    // New sites only in their companies.
    let site = json!({ "id": "branch", "name": "Branch" });
    let ok = w
        .hub
        .post_json("/api/companies/acme/sites", Some(&tech), site.clone())
        .await;
    assert_eq!(ok.status, StatusCode::OK);
    let no = w
        .hub
        .post_json("/api/companies/zeta/sites", Some(&tech), site)
        .await;
    assert_eq!(no.status, StatusCode::NOT_FOUND);
}
