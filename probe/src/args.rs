use common::cli;

/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: probe [OPTIONS]
       probe <install | uninstall | status | doctor>
       probe enroll <CODE>
       probe updates <on | off>

Blentinel Probe — silent network sentinel.
Runs the checks its hub gives it and ships encrypted reports to the hub.
Checks and the report interval are managed on the hub; the probe keeps
the last ones it received (config.json) for when the hub is unreachable.

Commands:
  install [--code <CODE>]
                  Install (or upgrade) as a service: copies itself into its
                  folder, enrolls with the code from the hub's \"Add probe\"
                  (a new code enrolls it again in place), starts the service.
                  Needs root / Administrator.
  uninstall [--delete-data]
                  Remove the service and the program. Keeps the identity and
                  enrollment unless --delete-data (then nothing is left).
  status          Service, hub, checks, reports waiting to be sent
  doctor          Check the folder, identity, enrollment, the hub's reachability
                  (through its pinned certificate) and the clock
  enroll <CODE>   Join a hub, then exit, without installing: writes
                  enrollment.json next to the probe.
  updates off     The local emergency brake: refuse every update the hub
                  offers (it shows the refusal) until `probe updates on`.
                  The probe updates itself only to releases signed with a
                  key built into it, newer than itself (see `status`).

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
  -v, --verbose   Log operational events to the terminal
                  (monitoring results, Hub connectivity)
  -d, --debug     Log cleartext payloads before encryption
                  (implies --verbose; never enable in production)
      --self-test Check this binary can run the probe in its folder, change
                  nothing, and exit (the updater runs it on a new version)

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
    /// `probe install`, `status`, …: run it, then exit.
    pub ops: Option<crate::ops::Command>,
    /// `--self-test`: check this binary can run here, then exit (D-55).
    pub self_test: bool,
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
            ops: None,
            self_test: false,
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
    let mut ops = None;
    let mut self_test = false;
    let common = cli::parse(args, |arg, rest| {
        match arg {
            "enroll" => enroll = Some(cli::value(arg, rest)?),
            "install" | "uninstall" | "status" | "doctor" => {
                ops = Some(parse_ops_command(arg, rest)?)
            }
            "updates" => {
                let on = match cli::value(arg, rest)?.as_str() {
                    "on" => true,
                    "off" => false,
                    other => {
                        return Err(cli::Exit::Error(format!(
                            "updates takes on or off, not {other}"
                        )));
                    }
                };
                ops = Some(crate::ops::Command::Updates { on });
            }
            "--self-test" => self_test = true,
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(Args {
        verbose: common.verbose,
        debug: common.debug,
        enroll,
        ops,
        self_test,
    })
}

/// Everything after `install`, `uninstall`, … belongs to it.
fn parse_ops_command(
    command: &str,
    rest: &mut impl Iterator<Item = String>,
) -> Result<crate::ops::Command, cli::Exit> {
    use crate::ops::Command;
    let (mut code, mut delete_data) = (None, false);
    while let Some(arg) = rest.next() {
        match (command, arg.as_str()) {
            ("install", "--code") => code = Some(cli::value(&arg, rest)?),
            ("uninstall", "--delete-data") => delete_data = true,
            (_, other) => {
                return Err(cli::Exit::Error(format!(
                    "Unknown option for {command}: {other}"
                )));
            }
        }
    }
    Ok(match command {
        "install" => Command::Install { code },
        "uninstall" => Command::Uninstall { delete_data },
        "status" => Command::Status,
        _ => Command::Doctor,
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
        assert_eq!(
            parse_from(["install".into(), "--code".into(), "BLT1-X".into()])
                .unwrap()
                .ops,
            Some(crate::ops::Command::Install {
                code: Some("BLT1-X".into())
            })
        );
        assert_eq!(
            parse_from(["uninstall".into(), "--delete-data".into()])
                .unwrap()
                .ops,
            Some(crate::ops::Command::Uninstall { delete_data: true })
        );
        for bad in [&["install", "--code"][..], &["status", "--code", "x"]] {
            assert!(matches!(
                parse_from(bad.iter().map(|s| s.to_string())),
                Err(cli::Exit::Error(_))
            ));
        }
        assert_eq!(
            parse_from(["updates".into(), "off".into()]).unwrap().ops,
            Some(crate::ops::Command::Updates { on: false })
        );
        assert!(parse_from(["updates".into(), "maybe".into()]).is_err());
        assert!(parse_from(["--self-test".into()]).unwrap().self_test);
        // The probe has no config file (D-49).
        for gone in ["--config", "--init"] {
            assert!(matches!(
                parse_from([gone.to_string()]),
                Err(cli::Exit::Error(_))
            ));
        }
    }
}
