#![cfg(feature = "ssr")]

pub mod monitor;

pub use monitor::spawn_db_size_monitor;
