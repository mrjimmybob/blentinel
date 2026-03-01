//! Logging infrastructure for the Blentinel Probe.
//!
//! Configures two independent output sinks:
//!
//! | Sink   | Level           | When active                          |
//! |--------|-----------------|--------------------------------------|
//! | File   | `DEBUG`+        | Always; level raised to DEBUG with `--debug` |
//! | Stdout | `INFO`+ / `DEBUG`+ | Only with `--verbose` or `--debug` |
//!
//! In normal (silent) operation only the file sink is active.  This keeps
//! service-mode probes quiet on the system console while still providing
//! full operational history on disk.
//!
//! The log file (`blentinel_probe.log`) lives next to the executable — the
//! same directory as `blentinel_probe.toml`, resolved via `config::get_base_dir()`.
//! Each run **appends** to the existing file; no rotation is applied.
//!
//! # Level matrix
//!
//! | Flags       | File   | Stdout |
//! |-------------|--------|--------|
//! | *(none)*    | INFO+  | OFF    |
//! | `--verbose` | INFO+  | INFO+  |
//! | `--debug`   | DEBUG+ | DEBUG+ |
//!
//! # Usage
//!
//! Call [`init_from_args`] once, early in `main()`, before any async runtime or
//! service dispatcher is started.  Hold the returned [`Guard`] for the entire
//! program lifetime — dropping it flushes and shuts down the background writer
//! thread.

use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::LevelFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

/// Holds the non-blocking file-writer thread alive.
///
/// Must be kept alive (e.g. `let _log_guard = logging::init_from_args(...)`) for the
/// full duration of the program.  When dropped it flushes all buffered log
/// entries and joins the background thread cleanly.
///
/// The `_writer` field is intentionally never read — it exists solely for its
/// `Drop` implementation.
pub struct Guard {
    _writer: WorkerGuard,
}

/// Initialise file + stdout logging from CLI flags.
///
/// This is the standard entry point.  It derives the appropriate sink levels
/// from the `--debug` and `--verbose` flags and delegates to [`init`].
///
/// See the [module-level level matrix](self) for the exact behaviour.
///
/// # Errors
///
/// Returns `Err` if the log file cannot be created/opened, or if a global
/// subscriber is already installed.  The caller should print a warning to
/// stderr and continue; the probe functions correctly without file logging.
pub fn init_from_args(debug: bool, verbose: bool) -> anyhow::Result<Guard> {
    let (file_level, stdout_level) = if debug {
        // --debug: full detail to both sinks so developers can correlate
        // console output with the on-disk record without switching tools.
        (LevelFilter::DEBUG, LevelFilter::DEBUG)
    } else if verbose {
        // --verbose: operational events on both sinks; file stays at INFO
        // to avoid overwhelming the log file with debug chatter.
        (LevelFilter::INFO, LevelFilter::INFO)
    } else {
        // Normal / service mode: console completely silent, file captures
        // everything INFO+ for post-hoc diagnosis.
        (LevelFilter::INFO, LevelFilter::OFF)
    };

    init(file_level, stdout_level)
}

/// Low-level logging initialiser with explicit sink levels.
///
/// Prefer [`init_from_args`] for the standard CLI use case.
/// This function is useful when you need precise control over both levels
/// independently (e.g. in tests or unusual deployment scenarios).
///
/// # Errors
///
/// Returns `Err` if the log file cannot be created/opened, or if a global
/// subscriber is already installed.
pub fn init(file_level: LevelFilter, stdout_level: LevelFilter) -> anyhow::Result<Guard> {
    let log_dir = crate::config::get_base_dir();

    // Non-blocking appender: a dedicated background OS thread handles all
    // file I/O, so disk writes never stall the async monitoring loop.
    // `Rotation::Never` produces a single file with exactly the given name.
    let file_appender = tracing_appender::rolling::never(&log_dir, "blentinel_probe.log");
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    // File layer — no ANSI escape codes, level controlled by caller.
    // Service processes cannot show stdout; this layer is the only way to
    // get operational insight when running under SCM.
    let file_layer = fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false)
        .with_target(false)
        .with_filter(file_level);

    // Stdout layer — compact, human-readable, level controlled by caller.
    // OFF in normal mode; INFO+ with --verbose; DEBUG+ with --debug.
    // In service mode the OS discards stdout regardless, making this a no-op.
    let stdout_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_target(false)
        .compact()
        .with_filter(stdout_level);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .try_init()
        .context("Failed to install tracing subscriber")?;

    Ok(Guard { _writer: guard })
}
