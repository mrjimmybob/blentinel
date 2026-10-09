use common::cli;

use crate::model::Role;

/// Compiled-in version from Cargo.toml — always in sync, zero runtime cost.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: hub [OPTIONS]
       hub [OPTIONS] <install | upgrade | uninstall | status | doctor | backup | restore>
       hub [OPTIONS] release <add <folder | file.tar.gz> | list>
       hub [OPTIONS] user <COMMAND>

Blentinel Hub — central collection and storage node.
Receives encrypted reports from probes, verifies signatures,
and persists data to the local database.

Options:
  -h, --help               Print this help message and exit
      --version            Print version and exit
      --init               Create a default blentinel_hub.toml and exit
      --config <path>      Path to the hub configuration TOML file (default:
                           blentinel_hub.toml in the working directory, else
                           the installed hub's)
      --print-public-key   Print the hub public key to stdout and write
                           hub_identity.pub, then exit (no server start)
  -v, --verbose            Debug logging: also one line per report received
  -d, --debug              Trace logging: also every accepted report in clear text
                           (implies --verbose; never enable in production)

Running the hub on this server:
  install [--host <ip>] [--port <n>] [--https-port <n>] [--no-service]
                     Install (or upgrade) as a service: copies itself into place,
                     creates the config (first time only; listens on 0.0.0.0,
                     3000 and 3443 unless given), restricts the data folder,
                     starts the service and prints the address and setup code.
                     Needs root / Administrator.
  upgrade [--from <url | folder>]
                     Upgrade the installed hub to a newer release: checks its
                     signature, runs the new hub's self-test on a copy of the
                     database, backs up, switches, and puts the old hub and
                     database back if the new one doesn't keep running. From
                     [upgrades] release_url (GitHub) unless --from. Adds the
                     release's probes. Needs root / Administrator.
  self-test          Check that this hub can run here: reads the config and
                     key, upgrades a copy of the database (`upgrade` runs it)
  uninstall [--delete-data]
                     Remove the service and program. Keeps the data and config
                     unless --delete-data. Needs root / Administrator.
  status             Service, ports, database, probes, certificate expiry
  doctor             Check config, permissions, keys, database, certificate,
                     ports and disk space, with what to do about problems
  backup [--out <file>]
                     Database (while the hub runs), keys, certificate and
                     config in one .tar.gz (default: the backups folder,
                     [backups] path, <data>/backups/ unless set)
  restore <file>     Put a backup back (the hub must be stopped; the files it
                     replaces are moved aside)
  release add <folder | file.tar.gz>
                     Add a signed probe release (`blentinelmake release build`);
                     the admin page approves it for probes to update to
  release list       The probe releases on this hub, newest first

User accounts (break-glass, on the server; works while the hub runs):
  user list                              List users
  user create <username> --role <role> [--display-name <name>]
                                         Create a user with a temporary password
                                         (printed once; changed at first login).
                                         Roles: owner, admin, technician, viewer
                                         (all companies; Clients: in the web UI)
  user reset-password <username>         New temporary password; logs the user out

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
    /// `hub user …`: manage accounts, then exit.
    pub user: Option<UserCommand>,
    /// `hub install`, `status`, …: run the hub's server administration, then exit.
    pub ops: Option<OpsCommand>,
    /// Optional path to the hub configuration TOML file.
    /// When `None`, the default path (`blentinel_hub.toml`) is used.
    pub config_path: Option<std::path::PathBuf>,
}

/// `hub user …` (proposal 0001 §4.4): the break-glass path when nobody can log in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCommand {
    List,
    Create {
        username: String,
        role: Role,
        display_name: Option<String>,
    },
    ResetPassword {
        username: String,
    },
}

/// Server administration commands (M2-08, D-52).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpsCommand {
    Install(crate::ops::install::InstallOptions),
    Uninstall {
        delete_data: bool,
    },
    Status,
    Doctor,
    Backup {
        out: Option<std::path::PathBuf>,
    },
    Restore {
        file: std::path::PathBuf,
    },
    /// `upgrade`: to a newer hub release (M4-04, D-75).
    Upgrade(crate::ops::hub_upgrade::UpgradeOptions),
    /// `self-test`: what `upgrade` runs on the new hub.
    SelfTest,
    /// `release add <path>`: a signed probe release (D-55).
    ReleaseAdd {
        path: std::path::PathBuf,
    },
    ReleaseList,
}

/// Parse the process arguments. `--help`, `--version` and usage errors print
/// and exit here, before an `Args` value exists.
pub fn parse() -> Args {
    parse_from(std::env::args().skip(1)).unwrap_or_else(|exit| exit.exit(VERSION, HELP))
}

/// Parse `args` (without the program name): the common flags
/// (`common::cli`) plus the hub's own options.
fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Args, cli::Exit> {
    let mut init = false;
    let mut print_public_key = false;
    let mut config_path = None;
    let mut user = None;
    let mut ops = None;
    let common = cli::parse(args, |arg, rest| {
        match arg {
            "--init" | "--create-config" => init = true,
            "--print-public-key" => print_public_key = true,
            "--config" => config_path = Some(std::path::PathBuf::from(cli::value(arg, rest)?)),
            // Everything after `user` belongs to it.
            "user" => user = Some(parse_user_command(rest, &mut config_path)?),
            "install" | "upgrade" | "self-test" | "uninstall" | "status" | "doctor" | "backup"
            | "restore" => {
                ops = Some(parse_ops_command(arg, rest, &mut config_path)?);
            }
            "release" => ops = Some(parse_release_command(rest, &mut config_path)?),
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(Args {
        verbose: common.verbose,
        debug: common.debug,
        init,
        print_public_key,
        user,
        ops,
        config_path,
    })
}

/// Everything after `install`, `uninstall`, … belongs to it.
fn parse_ops_command(
    command: &str,
    rest: &mut impl Iterator<Item = String>,
    config_path: &mut Option<std::path::PathBuf>,
) -> Result<OpsCommand, cli::Exit> {
    let usage = |message: String| cli::Exit::Error(message);
    let mut install = crate::ops::install::InstallOptions::default();
    let mut upgrade = crate::ops::hub_upgrade::UpgradeOptions::default();
    let (mut delete_data, mut out, mut file) = (false, None, None);
    let port = |option: &str, value: String| {
        value
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| usage(format!("{option} needs a port number, not {value:?}.")))
    };
    while let Some(arg) = rest.next() {
        match (command, arg.as_str()) {
            (_, "--config") => *config_path = Some(cli::value(&arg, rest)?.into()),
            ("install", "--host") => install.host = Some(cli::value(&arg, rest)?),
            ("install", "--port") => install.port = Some(port(&arg, cli::value(&arg, rest)?)?),
            ("install", "--https-port") => {
                install.https_port = Some(port(&arg, cli::value(&arg, rest)?)?)
            }
            ("install", "--no-service") => install.no_service = true,
            ("upgrade", "--from") => upgrade.from = Some(cli::value(&arg, rest)?),
            ("uninstall", "--delete-data") => delete_data = true,
            ("backup", "--out") => out = Some(cli::value(&arg, rest)?.into()),
            ("restore", path) if !path.starts_with('-') && file.is_none() => {
                file = Some(std::path::PathBuf::from(path))
            }
            (_, other) => return Err(usage(format!("Unknown option for {command}: {other}"))),
        }
    }
    Ok(match command {
        "install" => OpsCommand::Install(install),
        "upgrade" => OpsCommand::Upgrade(upgrade),
        "self-test" => OpsCommand::SelfTest,
        "uninstall" => OpsCommand::Uninstall { delete_data },
        "status" => OpsCommand::Status,
        "doctor" => OpsCommand::Doctor,
        "backup" => OpsCommand::Backup { out },
        _ => OpsCommand::Restore {
            file: file.ok_or_else(|| usage("restore needs the backup file.".into()))?,
        },
    })
}

/// `release add <path>` or `release list`.
fn parse_release_command(
    rest: &mut impl Iterator<Item = String>,
    config_path: &mut Option<std::path::PathBuf>,
) -> Result<OpsCommand, cli::Exit> {
    let usage =
        || cli::Exit::Error("Usage: hub release add <folder | file.tar.gz> | release list".into());
    let action = rest.next().ok_or_else(usage)?;
    let mut path = None;
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--config" => *config_path = Some(cli::value(&arg, rest)?.into()),
            p if action == "add" && path.is_none() && !p.starts_with('-') => path = Some(p.into()),
            _ => return Err(usage()),
        }
    }
    match (action.as_str(), path) {
        ("add", Some(path)) => Ok(OpsCommand::ReleaseAdd { path }),
        ("list", None) => Ok(OpsCommand::ReleaseList),
        _ => Err(usage()),
    }
}

fn parse_user_command(
    rest: &mut impl Iterator<Item = String>,
    config_path: &mut Option<std::path::PathBuf>,
) -> Result<UserCommand, cli::Exit> {
    let usage = |message: &str| cli::Exit::Error(message.to_string());
    let action = rest
        .next()
        .ok_or_else(|| usage("user needs a command: list, create or reset-password."))?;
    let (mut positional, mut role, mut display_name) = (Vec::new(), None, None);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--role" => {
                let value = cli::value(&arg, rest)?;
                role = Some(Role::parse(&value).ok_or_else(|| {
                    usage(&format!(
                        "Unknown role {value:?}: use owner, admin, technician or viewer."
                    ))
                })?);
            }
            "--display-name" => display_name = Some(cli::value(&arg, rest)?),
            "--config" => *config_path = Some(cli::value(&arg, rest)?.into()),
            option if option.starts_with('-') => {
                return Err(usage(&format!("Unknown option for user: {option}")));
            }
            _ => positional.push(arg),
        }
    }
    let username = |positional: Vec<String>| match <[String; 1]>::try_from(positional) {
        Ok([name]) => Ok(name),
        Err(_) => Err(usage(&format!("user {action} needs exactly one username."))),
    };
    match action.as_str() {
        "list" if positional.is_empty() => Ok(UserCommand::List),
        "create" => Ok(UserCommand::Create {
            username: username(positional)?,
            role: role.ok_or_else(|| usage("user create needs --role <role>."))?,
            display_name,
        }),
        "reset-password" => Ok(UserCommand::ResetPassword {
            username: username(positional)?,
        }),
        _ => Err(usage(&format!(
            "Unknown user command {action:?}: use list, create or reset-password."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Args, cli::Exit> {
        parse_from(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn release_commands_take_a_path_or_nothing() {
        assert_eq!(
            parse_args(&["release", "add", "rel.tar.gz"]).unwrap().ops,
            Some(OpsCommand::ReleaseAdd {
                path: "rel.tar.gz".into()
            })
        );
        let list = parse_args(&["release", "list", "--config", "/etc/h.toml"]).unwrap();
        assert_eq!(list.ops, Some(OpsCommand::ReleaseList));
        assert_eq!(list.config_path, Some("/etc/h.toml".into()));
        for bad in [
            &["release"][..],
            &["release", "add"],
            &["release", "list", "x"],
            &["release", "approve", "0.2.0"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
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
        assert!(args.user.is_none() && !args.init);
    }

    #[test]
    fn server_commands_take_their_own_options() {
        let install = parse_args(&["install", "--host", "10.0.0.5", "--https-port", "443"])
            .unwrap()
            .ops;
        assert_eq!(
            install,
            Some(OpsCommand::Install(crate::ops::install::InstallOptions {
                host: Some("10.0.0.5".into()),
                port: None,
                https_port: Some(443),
                no_service: false,
            }))
        );
        assert_eq!(
            parse_args(&["uninstall", "--delete-data"]).unwrap().ops,
            Some(OpsCommand::Uninstall { delete_data: true })
        );
        let upgrade = parse_args(&["upgrade", "--from", "/srv/rel"]).unwrap();
        assert_eq!(
            upgrade.ops,
            Some(OpsCommand::Upgrade(
                crate::ops::hub_upgrade::UpgradeOptions {
                    from: Some("/srv/rel".into())
                }
            ))
        );
        assert_eq!(
            parse_args(&["self-test", "--config", "/etc/h.toml"])
                .unwrap()
                .ops,
            Some(OpsCommand::SelfTest)
        );
        let restore = parse_args(&["restore", "b.tar.gz", "--config", "/etc/h.toml"]).unwrap();
        assert_eq!(
            restore.ops,
            Some(OpsCommand::Restore {
                file: "b.tar.gz".into()
            })
        );
        assert_eq!(
            restore.config_path.as_deref(),
            Some(std::path::Path::new("/etc/h.toml"))
        );
        for bad in [
            &["restore"][..],
            &["install", "--port", "http"],
            &["status", "--delete-data"],
            &["backup", "--out"],
        ] {
            assert!(
                matches!(parse_args(bad), Err(cli::Exit::Error(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn init_has_an_alias() {
        assert!(parse_args(&["--init"]).unwrap().init);
        assert!(parse_args(&["--create-config"]).unwrap().init);
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
    fn user_commands_parse() {
        let args = parse_args(&["--config", "hub.toml", "user", "list"]).unwrap();
        assert_eq!(args.user, Some(UserCommand::List));
        assert_eq!(
            args.config_path.as_deref(),
            Some(std::path::Path::new("hub.toml"))
        );

        let args = parse_args(&[
            "user",
            "create",
            "alice",
            "--role",
            "admin",
            "--display-name",
            "Alice A",
        ])
        .unwrap();
        assert_eq!(
            args.user,
            Some(UserCommand::Create {
                username: "alice".into(),
                role: Role::Admin,
                display_name: Some("Alice A".into()),
            })
        );

        let args = parse_args(&["user", "reset-password", "bob", "--config", "x.toml"]).unwrap();
        assert_eq!(
            args.user,
            Some(UserCommand::ResetPassword {
                username: "bob".into()
            })
        );
        assert_eq!(
            args.config_path.as_deref(),
            Some(std::path::Path::new("x.toml"))
        );
    }

    #[test]
    fn bad_user_commands_are_usage_errors() {
        for bad in [
            &["user"][..],
            &["user", "delete", "x"],
            &["user", "create", "alice"],
            &["user", "create", "alice", "--role", "root"],
            &["user", "create", "--role", "admin"],
            &["user", "reset-password", "a", "b"],
            &["user", "list", "extra"],
            &["user", "list", "--bogus"],
        ] {
            assert!(
                matches!(parse_args(bad), Err(cli::Exit::Error(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_help_text_lists_every_option() {
        for option in [
            "--init",
            "--config",
            "--print-public-key",
            "user create",
            "user reset-password",
            "user list",
            "--verbose",
            "--debug",
            "--version",
        ] {
            assert!(HELP.contains(option), "{option} missing from --help");
        }
    }
}
