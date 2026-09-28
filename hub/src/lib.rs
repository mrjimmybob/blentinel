#![recursion_limit = "512"]

// ---------------------------------------------------------------------------
// Front end (compiled for both the server and the WASM client)
// ---------------------------------------------------------------------------

pub mod app;
pub mod model;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
    // The page is interactive from here on. Browser tests (hub/end2end) wait
    // for this marker before clicking, instead of racing the WASM download.
    if let Some(root) = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-hydrated", "");
    }
}

// ---------------------------------------------------------------------------
// Server (ssr only). These live in the library, not in main.rs, so that
// integration tests in hub/tests/ can build the real router and state.
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
pub mod accounts;
#[cfg(feature = "ssr")]
pub mod api;
#[cfg(feature = "ssr")]
pub mod archive_viewer;
#[cfg(feature = "ssr")]
pub mod args;
#[cfg(feature = "ssr")]
pub mod auth;
#[cfg(feature = "ssr")]
pub mod clock;
#[cfg(feature = "ssr")]
pub mod companies;
#[cfg(feature = "ssr")]
pub mod config;
#[cfg(feature = "ssr")]
pub mod error;
#[cfg(feature = "ssr")]
pub mod hot_reload;
#[cfg(feature = "ssr")]
pub mod identity;
#[cfg(feature = "ssr")]
pub mod ingest;
#[cfg(feature = "ssr")]
pub mod logging;
#[cfg(feature = "ssr")]
pub mod probes;
#[cfg(feature = "ssr")]
pub mod routes;
#[cfg(feature = "ssr")]
pub mod server;
#[cfg(feature = "ssr")]
pub mod service;
#[cfg(feature = "ssr")]
pub mod state;
#[cfg(feature = "ssr")]
pub mod storage;
#[cfg(feature = "ssr")]
pub mod tls;
#[cfg(feature = "ssr")]
pub mod user_cli;

#[cfg(feature = "ssr")]
pub use state::AppState;
