#![cfg(feature = "ssr")]
//! Business rules (ENGINEERING.md §2.2): between the HTTP handlers (`api`,
//! `ingest`, …) and storage. Services know nothing about HTTP, CLI flags or
//! SQL. They take the storage traits and plain values, and return typed
//! results the handlers map to responses.

pub mod acme;
pub mod alerts;
pub mod audit;
pub mod backups;
pub mod checks;
pub mod commands;
pub mod companies;
pub mod dashboard;
pub mod dependencies;
pub mod digest;
pub mod discovery;
pub mod followups;
pub mod hub_upgrades;
pub mod incidents;
pub mod ingest;
pub mod inventory;
pub mod liveness;
pub mod login_limit;
pub mod maintenance;
pub mod notify;
pub mod poll;
pub mod probes;
pub mod releases;
pub mod retention;
pub mod rollout;
pub mod rotation;
pub mod scope;
pub mod setup;
pub mod silences;
pub mod templates;
pub mod thresholds;
pub mod users;
