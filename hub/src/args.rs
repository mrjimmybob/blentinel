use common::cli;

/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: hub [OPTIONS]

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
  -v, --verbose            Debug logging: also one line per report received
  -d, --debug              Trace logging: also every accepted report in clear text
                           (implies --verbose; never enable in production)

The log goes to stdout (under systemd: journalctl -u blentinel-hub).
RUST_LOG overrides the flags, e.g. RUST_LOG=hub_lib=debug,sqlx=warn
";

/// Resolved command-line flags after parsing.
///
/// `debug` implies `verbose` — both fields will be `true` when `--debug` is passed.
/// Consumers only need to check a single field; no flag-counting required.
#[derive(Debug, Clone, Default)]
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

/// Parse the process arguments. `--help`, `--version` and usage errors print
/// and exit here, before an `Args` value exists.
pub fn parse() -> Args {
    parse_from(std::env::args().skip(1)).unwrap_or_else(|exit| exit.exit(VERSION, HELP))
}

/// Parse `args` (without the program name): the common flags
/// (`common::cli`) plus the hub's own options.
fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Args, cli::Exit> {
    let mut print_public_key = false;
    let mut reset_admin_token = false;
    let mut config_path = None;
    let common = cli::parse(args, |arg, rest| {
        match arg {
            "--print-public-key" => print_public_key = true,
            "--reset-admin-token" => reset_admin_token = true,
            "--config" => config_path = Some(std::path::PathBuf::from(cli::value(arg, rest)?)),
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(Args {
        verbose: common.verbose,
        debug: common.debug,
        init: common.init,
        print_public_key,
        reset_admin_token,
        config_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Args, cli::Exit> {
        parse_from(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn hub_options_and_common_flags_combine() {
        let args = parse_args(&[
            "--config",
            "/etc/blentinel/hub.toml",
            "-d",
            "--print-public-key",
        ])
        .unwrap();
        assert_eq!(
            args.config_path.as_deref(),
            Some(std::path::Path::new("/etc/blentinel/hub.toml"))
        );
        assert!(args.debug && args.verbose && args.print_public_key);
        assert!(!args.reset_admin_token && !args.init);
    }

    #[test]
    fn config_needs_a_path_and_unknown_options_fail() {
        assert!(matches!(
            parse_args(&["--config"]),
            Err(cli::Exit::Error(_))
        ));
        assert!(matches!(
            parse_args(&["--config=x"]),
            Err(cli::Exit::Error(_))
        ));
    }

    #[test]
    fn the_help_text_lists_every_option() {
        for option in [
            "--init",
            "--config",
            "--print-public-key",
            "--reset-admin-token",
            "--verbose",
            "--debug",
            "--version",
        ] {
            assert!(HELP.contains(option), "{option} missing from --help");
        }
    }
}
