/// The hub's signed, sealed answer to a report (protocol v3, D-48).
#[cfg(feature = "crypto")]
pub mod ack;
/// Checks, the probe's config and check results (M2-04, D-49).
pub mod checks;
pub mod cli;
/// The command channel: hub → probe commands and their results (M2-06, D-56).
pub mod commands;
pub mod enrollment;
pub mod fs;
/// Hub releases: what `hub.sh` and `hub upgrade` download (M2-10, M4-04).
#[cfg(feature = "crypto")]
pub mod hub_release;
pub mod models;
/// What a probe pins of its hub, and how the hub changes it (M2-13, D-61).
#[cfg(feature = "crypto")]
pub mod pins;
pub mod protocol;
/// Signed probe releases (M2-15, D-55).
#[cfg(feature = "crypto")]
pub mod release;

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
