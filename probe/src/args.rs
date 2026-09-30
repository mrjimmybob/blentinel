use common::cli;

/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: probe [OPTIONS]
       probe enroll <CODE>

Blentinel Probe — silent network sentinel.
Runs the checks its hub gives it and ships encrypted reports to the hub.
Checks and the report interval are managed on the hub; the probe keeps
the last ones it received (config.json) for when the hub is unreachable.

Commands:
  enroll <CODE>   Join a hub with the code from its \"Add probe\" (web UI),
                  then exit. Writes enrollment.json next to the probe; run
                  it again with a new code to move to another hub or company.

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
  -v, --verbose   Log operational events to the terminal
                  (monitoring results, Hub connectivity)
  -d, --debug     Log cleartext payloads before encryption
                  (implies --verbose; never enable in production)

On Windows the probe auto-detects whether it was launched by the Service
Control Manager and runs as a service in that case, or as a normal CLI
process otherwise.  No flag is required for either mode.
";

/// Resolved command-line flags after parsing.
///
/// `debug` implies `verbose` — both fields will be `true` when `--debug` is passed.
/// Consumers only need to check a single field; no flag-counting required.
#[derive(Debug, Clone)]
pub struct Args {
    /// Log operational events to the terminal.
    pub verbose: bool,
    /// Log cleartext payloads before encryption (strict superset of verbose).
    pub debug: bool,
    /// `probe enroll <CODE>`: redeem an enrollment code, then exit.
    pub enroll: Option<String>,
}

impl Args {
    /// Silent defaults suitable for running as a background service.
    ///
    /// Services have no interactive terminal, so verbose/debug output is
    /// suppressed. Enrollment is done from the CLI.
    #[cfg(windows)] // only the Windows service entry point uses it
    pub fn service_defaults() -> Self {
        Self {
            verbose: false,
            debug: false,
            enroll: None,
        }
    }
}

/// Parse the process arguments. `--help`, `--version` and usage errors print
/// and exit here, before an `Args` value exists.
pub fn parse() -> Args {
    parse_from(std::env::args().skip(1)).unwrap_or_else(|exit| exit.exit(VERSION, HELP))
}

/// The common flags (`common::cli`) plus `enroll <CODE>`.
fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Args, cli::Exit> {
    let mut enroll = None;
    let common = cli::parse(args, |arg, rest| {
        if arg == "enroll" {
            enroll = Some(cli::value(arg, rest)?);
            return Ok(true);
        }
        Ok(false)
    })?;
    Ok(Args {
        verbose: common.verbose,
        debug: common.debug,
        enroll,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_flags_are_accepted_and_others_rejected() {
        let args = parse_from(["--debug".to_string()]).unwrap();
        assert!(args.debug && args.verbose);
        assert_eq!(args.enroll, None);
        let enroll = parse_from(["enroll".to_string(), "BLT1-XYZ".to_string()]).unwrap();
        assert_eq!(enroll.enroll.as_deref(), Some("BLT1-XYZ"));
        assert!(matches!(
            parse_from(["enroll".to_string()]),
            Err(cli::Exit::Error(_))
        ));
        // The probe has no config file (D-49).
        for gone in ["--config", "--init"] {
            assert!(matches!(
                parse_from([gone.to_string()]),
                Err(cli::Exit::Error(_))
            ));
        }
    }
}
