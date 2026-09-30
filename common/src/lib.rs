/// The hub's signed, sealed answer to a report (protocol v3, D-48).
#[cfg(feature = "crypto")]
pub mod ack;
/// Checks, the probe's config and check results (M2-04, D-49).
pub mod checks;
pub mod cli;
pub mod enrollment;
pub mod fs;
pub mod models;
pub mod protocol;

/// Probe ↔ hub sealing and signatures. Behind the `crypto` feature so the
/// hub's WASM front end (which also depends on `common`) doesn't pull in
/// OS randomness it can't use.
#[cfg(feature = "crypto")]
pub mod crypto;

/// What the hub and the probe need to install themselves as services.
#[cfg(feature = "service")]
pub mod service;

/// Config-file watching for hot reload. Behind the `watch` feature (needs tokio).
#[cfg(feature = "watch")]
pub mod watch;
