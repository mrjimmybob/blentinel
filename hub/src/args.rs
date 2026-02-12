/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

// let exe = std::env::args().next().unwrap_or("blentinel_probe".into());
// println!("Usage: {} [OPTIONS]", exe);

const HELP: &str = "\
Usage: blentinel_hub [OPTIONS]

Blentinel Hub — central collection and storage node.
Receives encrypted reports from probes, verifies signatures,
and persists data to the local database.

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
      --init      Create a default blentinel_hub.toml and exit
  -v, --verbose   Log operational events to the terminal
                  (probe reports received, data saved to DB)
  -d, --debug     Verbose mode + detailed diagnostic output
                  (implies --verbose; never enable in production)
";

/// Resolved command-line flags after parsing.
///
/// `debug` implies `verbose` — both fields will be `true` when `--debug` is passed.
/// Consumers only need to check a single field; no flag-counting required.
#[derive(Debug, Clone)]
pub struct Args {
    /// Log operational events to the terminal.
    pub verbose: bool,
    /// Log detailed diagnostic output (strict superset of verbose).
    pub debug: bool,
    /// Create a default configuration file and exit.
    pub init: bool,
}

/// Parse `std::env::args` into a resolved `Args` struct.
///
/// `--version` and `--help` are handled here: they print to stdout and
/// terminate with exit code 0 before an `Args` value is ever constructed.
/// An unrecognised flag prints a short error to stderr and exits with code 1.
pub fn parse() -> Args {
    let mut verbose = false;
    let mut debug = false;
    let mut init = false;

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
            "--init" | "--create-config" => init = true,
            "-v" | "--verbose" => verbose = true,
            "-d" | "--debug" => debug = true,
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

    Args { verbose, debug, init }
}
