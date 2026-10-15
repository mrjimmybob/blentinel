#![cfg(feature = "ssr")]
//! Shared application state handed to every Axum handler.

use std::sync::{Arc, PoisonError};

use leptos::prelude::LeptosOptions;
use tokio::sync::{Notify, RwLock};

use crate::service::alerts::Notifier;
use crate::{args, clock::Clock, config, storage::Storage};

#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    /// The database, behind the storage traits (never a concrete backend).
    /// Replaced when `state_dir` / `db_path` change (D-38).
    pub storage: Reloadable<Arc<dyn Storage>>,
    /// Replaced when `identity_key_path` changes (D-38).
    pub hub_secret: Reloadable<x25519_dalek::StaticSecret>,
    pub config: Arc<RwLock<config::HubConfig>>,
    pub args: args::Args,
    /// The first-run setup code, while the database has no users (D-42).
    pub setup: Arc<crate::service::setup::SetupCode>,
    /// Failed logins and setup codes, to slow down guessing (D-43).
    pub login_limiter: Arc<crate::service::login_limit::LoginLimiter>,
    pub clock: Arc<dyn Clock>,
    /// Where alerts go: the routing rules and channels in production
    /// (`service::notify::Dispatcher`); tests record.
    pub notifier: Arc<dyn Notifier>,
    /// Delivering to channels, and sealing their secrets (D-66).
    pub delivery: Arc<crate::service::notify::Delivery>,
    /// Woken after a reload changed where the hub listens (`crate::server`).
    pub listeners_changed: Arc<Notify>,
    /// The release keys it trusts (`common::release::trusted_keys`; tests
    /// use their own): a probe release signed otherwise isn't taken (D-55).
    pub release_keys: Arc<Vec<[u8; 32]>>,
    /// The hub's pins as it tells probes: current, next while a rotation
    /// announces, and the previous key (D-61).
    pub hub_pins: Reloadable<crate::keys::Credentials>,
    /// The HTTPS listener's certificate, replaced in place by a switch.
    pub tls: crate::tls::TlsHandle,
    /// The ACME authority the hub gets its certificate from (D-62; tests
    /// use their own).
    pub acme: Arc<crate::acme::Acme>,
    /// One-time links for downloading backups (D-60).
    pub backup_downloads: Arc<crate::service::backups::DownloadTickets>,
    /// Command polls being held, to answer when a command is queued (D-76).
    pub command_waiters: Arc<crate::service::poll::CommandWaiters>,
    /// The config file, which backups include (D-60).
    pub config_path: std::path::PathBuf,
}

/// A value a config reload can replace while the hub runs (D-38).
///
/// `get` returns the current value (cheap for `Arc`s). A value being replaced
/// stays alive until the requests already using it are done.
pub struct Reloadable<T>(Arc<std::sync::RwLock<T>>);

impl<T: Clone> Reloadable<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(std::sync::RwLock::new(value)))
    }

    pub fn get(&self) -> T {
        // Poison-tolerant: every write is a single assignment.
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set(&self, value: T) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = value;
    }
}

// Manual: a derive would require `T: Clone` for cloning the handle.
impl<T> Clone for Reloadable<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
