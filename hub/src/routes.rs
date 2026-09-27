#![cfg(feature = "ssr")]
//! The hub's HTTP API route table: the single place where paths meet handlers.
//!
//! `main` adds the Leptos UI routes on top. Tests use [`api_router`] directly,
//! so the API can be exercised in-process without a browser, a socket, or the
//! compiled WASM front end.

use axum::{
    routing::{get, post},
    Router,
};

use crate::{api, archive_viewer, ingest, AppState};

/// Every `/api/*` route, without state attached.
pub fn api_router() -> Router<AppState> {
    Router::new()
        // Probe endpoints (no session; probes authenticate cryptographically)
        .route("/api/handshake", get(ingest::handle_handshake))
        .route("/api/report", post(ingest::handle_probe_report))
        // Auth
        .route("/api/login", post(api::login))
        .route("/api/logout", post(api::logout))
        // Dashboard & detail (require session)
        .route("/api/dashboard/companies", get(api::dashboard_companies))
        .route("/api/company/{company_id}/probes", get(api::company_probes))
        .route("/api/company/{company_id}/uptime", get(api::company_uptime))
        .route("/api/probe/{probe_id}/devices", get(api::probe_devices))
        // Admin
        .route("/api/admin/companies", get(api::admin_companies))
        .route("/api/admin/probes", get(api::admin_probes))
        .route(
            "/api/admin/delete-company-data",
            post(api::admin_delete_company_data),
        )
        .route(
            "/api/admin/delete-probe-data",
            post(api::admin_delete_probe_data),
        )
        .route("/api/admin/remove-probe", post(api::admin_remove_probe))
        .route("/api/admin/storage-info", get(api::admin_storage_info))
        .route("/api/admin/archives", get(api::admin_archives))
        .route("/api/admin/archive", post(api::admin_create_archive))
        // Alert silence management
        .route("/api/silences", get(api::list_silences))
        .route("/api/silence", post(api::create_silence))
        .route("/api/silence/delete", post(api::delete_silence))
        // Archive viewer (read-only historical access)
        .route(
            "/api/archive/{archive_id}/companies",
            get(archive_viewer::archive_companies),
        )
        .route(
            "/api/archive/{archive_id}/company/{company_id}/probes",
            get(archive_viewer::archive_company_probes),
        )
        .route(
            "/api/archive/{archive_id}/company/{company_id}/uptime",
            get(archive_viewer::archive_company_uptime),
        )
        .route(
            "/api/archive/{archive_id}/probe/{probe_id}/devices",
            get(archive_viewer::archive_probe_devices),
        )
}
