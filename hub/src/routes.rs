#![cfg(feature = "ssr")]
//! The hub's HTTP API route table: the single place where paths meet handlers,
//! and where each route declares who may call it ([`Access`], D-42).
//!
//! A route can't be added without an access level, and `hub/tests/access.rs`
//! checks every entry of [`access_table`] against its declaration, so a new
//! route can't silently skip authorization.
//!
//! `main` adds the Leptos UI routes on top. Tests use [`api_router`] directly,
//! so the API can be exercised in-process without a browser, a socket, or the
//! compiled WASM front end.

use axum::{
    Router,
    http::Method,
    middleware,
    routing::{MethodRouter, get, post},
};

use crate::auth::{self, Access};
use crate::model::Permission::{
    ManageCompanies, ManageData, ManageUsers, Silence, ViewAdmin, ViewData,
};
use crate::{AppState, accounts, api, archive_viewer, checks, companies, ingest, probes, releases};

/// One API route and what it requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAccess {
    pub method: Method,
    pub path: &'static str,
    pub access: Access,
}

const GET: Method = Method::GET;
const POST: Method = Method::POST;

/// Every `/api/*` route with its handler.
fn routes() -> Vec<(RouteAccess, MethodRouter<AppState>)> {
    use Access::{LoggedIn, Perm, Probe, Public};
    let route = |method: Method, path, access, handler| {
        (
            RouteAccess {
                method,
                path,
                access,
            },
            handler,
        )
    };
    vec![
        // Probes (no session: they authenticate cryptographically)
        route(
            POST,
            "/api/report",
            Probe,
            post(ingest::handle_probe_report),
        ),
        route(POST, "/api/enroll", Probe, post(probes::enroll)),
        // Login and first run
        route(POST, "/api/auth/login", Public, post(accounts::login)),
        route(POST, "/api/auth/logout", LoggedIn, post(accounts::logout)),
        route(GET, "/api/auth/me", LoggedIn, get(accounts::me)),
        route(GET, "/api/setup", Public, get(accounts::setup_status)),
        route(POST, "/api/setup", Public, post(accounts::setup)),
        // Own account
        route(
            POST,
            "/api/me/password",
            LoggedIn,
            post(accounts::change_password),
        ),
        route(
            POST,
            "/api/me/logout-others",
            Perm(ViewData),
            post(accounts::logout_others),
        ),
        // User management
        route(
            GET,
            "/api/users",
            Perm(ManageUsers),
            get(accounts::list_users),
        ),
        route(
            POST,
            "/api/users",
            Perm(ManageUsers),
            post(accounts::create_user),
        ),
        route(
            POST,
            "/api/users/{id}",
            Perm(ManageUsers),
            post(accounts::update_user),
        ),
        route(
            POST,
            "/api/users/{id}/reset-password",
            Perm(ManageUsers),
            post(accounts::reset_password),
        ),
        route(
            POST,
            "/api/users/{id}/logout",
            Perm(ManageUsers),
            post(accounts::force_logout),
        ),
        // Companies and sites (records, D-44; created by hand, D-47)
        route(GET, "/api/companies", Perm(ViewData), get(companies::list)),
        route(
            POST,
            "/api/companies",
            Perm(ManageData),
            post(companies::create),
        ),
        route(
            GET,
            "/api/companies/{id}",
            Perm(ViewData),
            get(companies::detail),
        ),
        route(
            POST,
            "/api/companies/{id}",
            Perm(ManageCompanies),
            post(companies::update),
        ),
        route(
            POST,
            "/api/companies/{id}/archive",
            Perm(ManageData),
            post(companies::archive),
        ),
        route(
            POST,
            "/api/companies/{id}/sites/{site}",
            Perm(ManageCompanies),
            post(companies::update_site),
        ),
        route(
            POST,
            "/api/companies/{id}/sites",
            Perm(ManageCompanies),
            post(companies::create_site),
        ),
        // Probes and enrollment codes (D-47)
        route(GET, "/api/probes", Perm(ViewData), get(probes::list)),
        route(
            POST,
            "/api/probes/{id}",
            Perm(ManageCompanies),
            post(probes::update),
        ),
        route(
            POST,
            "/api/probes/{id}/revoke",
            Perm(ManageData),
            post(probes::revoke),
        ),
        // Checks (D-49)
        route(
            GET,
            "/api/probes/{id}/checks",
            Perm(ViewData),
            get(checks::list),
        ),
        route(
            POST,
            "/api/probes/{id}/checks",
            Perm(ManageCompanies),
            post(checks::create),
        ),
        route(
            POST,
            "/api/probes/{id}/checks/copy",
            Perm(ManageCompanies),
            post(checks::copy),
        ),
        route(
            POST,
            "/api/checks/{id}",
            Perm(ManageCompanies),
            post(checks::update),
        ),
        route(
            POST,
            "/api/checks/{id}/delete",
            Perm(ManageCompanies),
            post(checks::delete),
        ),
        // Probe binaries (D-53): the wizard's list, and the download itself
        // (no session: the machine being set up has none).
        route(
            GET,
            "/api/probe-downloads",
            Perm(ManageCompanies),
            get(releases::list_downloads),
        ),
        // A probe endpoint: served on the plain-HTTP port too, where Windows
        // PowerShell 5.1 can't follow a 308 to HTTPS (D-53).
        route(
            GET,
            "/download/probe/{target}",
            Probe,
            get(releases::download),
        ),
        // A probe updating itself (D-55): a file of a signed release.
        route(
            GET,
            "/download/releases/{version}/{file}",
            Probe,
            get(releases::download_release_file),
        ),
        route(
            GET,
            "/api/enrollment-codes",
            Perm(ManageCompanies),
            get(probes::list_codes),
        ),
        route(
            POST,
            "/api/enrollment-codes",
            Perm(ManageCompanies),
            post(probes::create_code),
        ),
        route(
            POST,
            "/api/enrollment-codes/{id}/revoke",
            Perm(ManageCompanies),
            post(probes::revoke_code),
        ),
        // Dashboard & detail
        route(
            GET,
            "/api/dashboard/companies",
            Perm(ViewData),
            get(api::dashboard_companies),
        ),
        route(
            GET,
            "/api/company/{company_id}/probes",
            Perm(ViewData),
            get(api::company_probes),
        ),
        route(
            GET,
            "/api/company/{company_id}/uptime",
            Perm(ViewData),
            get(api::company_uptime),
        ),
        route(
            GET,
            "/api/probe/{probe_id}/devices",
            Perm(ViewData),
            get(api::probe_devices),
        ),
        // Admin
        route(
            GET,
            "/api/admin/companies",
            Perm(ViewAdmin),
            get(api::admin_companies),
        ),
        route(
            GET,
            "/api/admin/probes",
            Perm(ViewAdmin),
            get(api::admin_probes),
        ),
        route(
            POST,
            "/api/admin/delete-company-data",
            Perm(ManageData),
            post(api::admin_delete_company_data),
        ),
        route(
            POST,
            "/api/admin/delete-probe-data",
            Perm(ManageData),
            post(api::admin_delete_probe_data),
        ),
        route(
            POST,
            "/api/admin/remove-probe",
            Perm(ManageData),
            post(api::admin_remove_probe),
        ),
        route(
            GET,
            "/api/admin/storage-info",
            Perm(ViewAdmin),
            get(api::admin_storage_info),
        ),
        route(
            GET,
            "/api/admin/archives",
            Perm(ViewAdmin),
            get(api::admin_archives),
        ),
        route(
            POST,
            "/api/admin/archive",
            Perm(ManageData),
            post(api::admin_create_archive),
        ),
        // Probe releases and updates (D-55)
        route(
            GET,
            "/api/admin/probe-updates",
            Perm(ViewAdmin),
            get(releases::overview),
        ),
        route(
            POST,
            "/api/admin/probe-updates/approve",
            Perm(ManageData),
            post(releases::approve),
        ),
        route(
            POST,
            "/api/admin/probe-releases",
            Perm(ManageData),
            post(releases::add).layer(axum::extract::DefaultBodyLimit::max(
                crate::service::releases::MAX_ARCHIVE_BYTES,
            )),
        ),
        route(
            POST,
            "/api/admin/probe-releases/{version}/delete",
            Perm(ManageData),
            post(releases::remove),
        ),
        // Audit log (who did what; Admins)
        route(GET, "/api/audit", Perm(ViewAdmin), get(api::audit_log)),
        route(GET, "/api/audit.csv", Perm(ViewAdmin), get(api::audit_csv)),
        // Alert silences
        route(
            GET,
            "/api/silences",
            Perm(ViewData),
            get(api::list_silences),
        ),
        route(
            POST,
            "/api/silence",
            Perm(Silence),
            post(api::create_silence),
        ),
        route(
            POST,
            "/api/silence/delete",
            Perm(Silence),
            post(api::delete_silence),
        ),
        // Archive viewer (read-only historical access)
        route(
            GET,
            "/api/archive/{archive_id}/companies",
            Perm(ViewData),
            get(archive_viewer::archive_companies),
        ),
        route(
            GET,
            "/api/archive/{archive_id}/company/{company_id}/probes",
            Perm(ViewData),
            get(archive_viewer::archive_company_probes),
        ),
        route(
            GET,
            "/api/archive/{archive_id}/company/{company_id}/uptime",
            Perm(ViewData),
            get(archive_viewer::archive_company_uptime),
        ),
        route(
            GET,
            "/api/archive/{archive_id}/probe/{probe_id}/devices",
            Perm(ViewData),
            get(archive_viewer::archive_probe_devices),
        ),
    ]
}

/// Every API route and its declared access (for the access tests and docs).
pub fn access_table() -> Vec<RouteAccess> {
    routes().into_iter().map(|(spec, _)| spec).collect()
}

/// Every `/api/*` route, each behind [`auth::enforce`] for its access level.
/// Needs the state because the check reads sessions from storage.
pub fn api_router(state: &AppState) -> Router<AppState> {
    routes()
        .into_iter()
        .fold(Router::new(), |router, (spec, handler)| {
            let access = spec.access;
            let guard = middleware::from_fn_with_state(state.clone(), move |s, req, next| {
                auth::enforce(s, access, req, next)
            });
            router.route(spec.path, handler.route_layer(guard))
        })
}
