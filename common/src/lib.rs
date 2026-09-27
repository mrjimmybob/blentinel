pub mod models;
pub mod protocol;

/// Probe ↔ hub sealing and signatures. Behind the `crypto` feature so the
/// hub's WASM front end (which also depends on `common`) doesn't pull in
/// OS randomness it can't use.
#[cfg(feature = "crypto")]
pub mod crypto;
