#![cfg(feature = "ssr")]
//! Hub logging with `tracing`, to stdout (systemd puts it in the journal).
//!
//! | Flag | Level | Adds |
//! |---|---|---|
//! | (none) | info | startup, config reloads, rejected reports, alerts, archiving, errors |
//! | `--verbose` | debug | one line per accepted report, handshakes |
//! | `--debug` | trace | full decrypted reports |
//!
//! `RUST_LOG` overrides the flags (e.g. `RUST_LOG=hub_lib=debug,sqlx=warn`).
//! Other crates log at `warn` and above by default.

use crate::args::Args;
use tracing_subscriber::EnvFilter;

/// Install the global subscriber. Call once, first thing in `main`.
pub fn init(args: &Args) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter(args)));
    // Colours only on a terminal: under systemd or when redirected to a file,
    // escape codes would end up in the journal/log as garbage.
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());
    // `try_init`: a second call (tests) is harmless instead of a panic.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(ansi)
        .try_init();
}

/// The filter used when `RUST_LOG` is unset.
fn default_filter(args: &Args) -> String {
    let level = if args.debug {
        "trace"
    } else if args.verbose {
        "debug"
    } else {
        "info"
    };
    format!("warn,hub={level},hub_lib={level}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(verbose: bool, debug: bool) -> Args {
        Args {
            verbose,
            debug,
            ..Args::default()
        }
    }

    #[test]
    fn flags_raise_the_hub_level_only() {
        assert_eq!(
            default_filter(&args(false, false)),
            "warn,hub=info,hub_lib=info"
        );
        assert_eq!(
            default_filter(&args(true, false)),
            "warn,hub=debug,hub_lib=debug"
        );
        assert_eq!(
            default_filter(&args(true, true)),
            "warn,hub=trace,hub_lib=trace"
        );
    }
}
