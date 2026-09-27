#![cfg(feature = "ssr")]
//! Shared application state handed to every Axum handler.

use std::sync::Arc;

use leptos::prelude::LeptosOptions;
use tokio::sync::RwLock;

use crate::{args, auth, clock::Clock, config, storage::Storage};

#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    /// The database, behind the storage traits (never a concrete backend).
    pub storage: Arc<dyn Storage>,
    pub hub_secret: x25519_dalek::StaticSecret,
    pub config: Arc<RwLock<config::HubConfig>>,
    pub args: args::Args,
    pub sessions: auth::SessionStore,
    pub admin_token: String,
    pub clock: Arc<dyn Clock>,
}
