#![cfg(feature = "ssr")]
//! Business rules (ENGINEERING.md §2.2): between the HTTP handlers (`api`,
//! `ingest`, …) and storage. Services know nothing about HTTP, CLI flags or
//! SQL. They take the storage traits and plain values, and return typed
//! results the handlers map to responses.

pub mod alerts;
pub mod companies;
pub mod dashboard;
pub mod ingest;
pub mod liveness;
pub mod login_limit;
pub mod retention;
pub mod scope;
pub mod setup;
pub mod silences;
pub mod users;
