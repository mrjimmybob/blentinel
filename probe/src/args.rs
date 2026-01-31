/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: blentinel_probe [OPTIONS]

Blentinel Probe — silent network sentinel.
Monitors configured resources and ships encrypted reports to the Hub.

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
  -v, --verbose   Log operational events to the terminal
                  (handshake, monitoring results, Hub connectivity)
  -d, --debug     Log cleartext payloads before encryption
                  (implies --verbose; never enable in production)
      --daemon    Run as a background process (detached from terminal)
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
    /// Run detached from the controlling terminal (not yet implemented).
    #[allow(dead_code)]
    pub daemon: bool,
}

/// Parse `std::env::args` into a resolved `Args` struct.
///
/// `--version` and `--help` are handled here: they print to stdout and
/// terminate with exit code 0 before an `Args` value is ever constructed.
/// An unrecognised flag prints a short error to stderr and exits with code 1.
pub fn parse() -> Args {
    let mut verbose = false;
    let mut debug = false;
    let mut daemon = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" => {
                println!("{}", VERSION);
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print!("{}", HELP);
                std::process::exit(0);
            }
            "-v" | "--verbose" => verbose = true,
            "-d" | "--debug" => debug = true,
            "--daemon" => daemon = true,
            other => {
                eprintln!("Unknown option: {}\nRun with --help for usage information.", other);
                std::process::exit(1);
            }
        }
    }

    // --debug is a strict superset of --verbose
    if debug {
        verbose = true;
    }

    Args {
        verbose,
        debug,
        daemon,
    }
}
