/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: blentinel_hub [OPTIONS]

Blentinel Hub — central collection and storage node.
Receives encrypted reports from probes, verifies signatures,
and persists data to the local database.

Options:
  -h, --help               Print this help message and exit
      --version            Print version and exit
      --init               Create a default blentinel_hub.toml and exit
      --config <path>      Path to the hub configuration TOML file
                           (default: blentinel_hub.toml in the working directory)
      --print-public-key   Print the hub public key to stdout and write
                           hub_identity.pub, then exit (no server start)
      --reset-admin-token  Generate a new admin token, write hub_auth.token,
                           and print it to stdout, then exit (no server start)
  -v, --verbose            Log operational events to the terminal
                           (probe reports received, data saved to DB)
  -d, --debug              Verbose mode + detailed diagnostic output
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
    /// Print the hub public key to stdout and file, then exit.
    pub print_public_key: bool,
    /// Generate and write a new admin token, then exit.
    pub reset_admin_token: bool,
    /// Optional path to the hub configuration TOML file.
    /// When `None`, the default path (`blentinel_hub.toml`) is used.
    pub config_path: Option<std::path::PathBuf>,
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
    let mut print_public_key = false;
    let mut reset_admin_token = false;
    let mut config_path: Option<std::path::PathBuf> = None;

    // Peekable so `--config <path>` can consume the next token as its value.
    let mut iter = std::env::args().skip(1).peekable();
    while let Some(arg) = iter.next() {
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
            "--print-public-key" => print_public_key = true,
            "--reset-admin-token" => reset_admin_token = true,
            "--config" => {
                match iter.next() {
                    Some(path) => config_path = Some(std::path::PathBuf::from(path)),
                    None => {
                        eprintln!("--config requires a path argument.\nRun with --help for usage information.");
                        std::process::exit(1);
                    }
                }
            }
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

    Args { verbose, debug, init, print_public_key, reset_admin_token, config_path }
}
