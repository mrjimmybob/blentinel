//! Command-line flags shared by the probe and the hub, and one convention for
//! help, version and errors. Each binary adds its own options.
//!
//! Parsing is pure (it never prints or exits), so it can be tested; the
//! binary turns an [`Exit`] into output and an exit code with [`Exit::exit`].

/// Flags every Blentinel binary accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommonFlags {
    /// `-v`, `--verbose`
    pub verbose: bool,
    /// `-d`, `--debug`; implies `verbose`.
    pub debug: bool,
}

/// Why parsing ended without options to run with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// `-h`, `--help`
    Help,
    /// `--version`
    Version,
    /// A usage error, already worded for the user.
    Error(String),
}

impl Exit {
    /// Print what this means and end the process: help and version on stdout
    /// with code 0, errors on stderr with code 1.
    pub fn exit(self, version: &str, help: &str) -> ! {
        match self {
            Exit::Help => {
                print!("{help}");
                std::process::exit(0)
            }
            Exit::Version => {
                println!("{version}");
                std::process::exit(0)
            }
            Exit::Error(message) => {
                eprintln!("{message}\nRun with --help for usage information.");
                std::process::exit(1)
            }
        }
    }
}

/// Parse `args` (without the program name). The common flags are handled
/// here; every other argument is offered to `other`, which returns `Ok(true)`
/// if it recognised it (it may take a value from the iterator) and `Ok(false)`
/// if not.
pub fn parse<I, F>(args: I, mut other: F) -> Result<CommonFlags, Exit>
where
    I: IntoIterator<Item = String>,
    F: FnMut(&str, &mut I::IntoIter) -> Result<bool, Exit>,
{
    let mut flags = CommonFlags::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(Exit::Help),
            "--version" => return Err(Exit::Version),
            "-v" | "--verbose" => flags.verbose = true,
            "-d" | "--debug" => flags.debug = true,
            unknown => {
                if !other(unknown, &mut args)? {
                    return Err(Exit::Error(format!("Unknown option: {unknown}")));
                }
            }
        }
    }
    flags.verbose |= flags.debug;
    Ok(flags)
}

/// The value after an option such as `--config <path>`, or a usage error.
pub fn value(option: &str, args: &mut impl Iterator<Item = String>) -> Result<String, Exit> {
    args.next()
        .ok_or_else(|| Exit::Error(format!("{option} requires a value.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common(args: &[&str]) -> Result<CommonFlags, Exit> {
        parse(args.iter().map(|a| a.to_string()), |_, _| Ok(false))
    }

    #[test]
    fn no_flags_is_all_off() {
        assert_eq!(common(&[]), Ok(CommonFlags::default()));
    }

    #[test]
    fn debug_implies_verbose() {
        let flags = common(&["--debug"]).unwrap();
        assert!(flags.debug && flags.verbose);
        assert!(!common(&["-v"]).unwrap().debug);
    }

    #[test]
    fn help_and_version_stop_parsing() {
        assert_eq!(common(&["-v", "--help", "--bogus"]), Err(Exit::Help));
        assert_eq!(common(&["--version"]), Err(Exit::Version));
    }

    #[test]
    fn unknown_options_are_errors() {
        assert_eq!(
            common(&["--bogus"]),
            Err(Exit::Error("Unknown option: --bogus".to_string()))
        );
    }

    #[test]
    fn a_binary_can_add_options_with_values() {
        let mut path = None;
        let flags = parse(
            ["--config", "/etc/x.toml", "-v"].map(String::from),
            |arg, args| match arg {
                "--config" => {
                    path = Some(value(arg, args)?);
                    Ok(true)
                }
                _ => Ok(false),
            },
        )
        .unwrap();
        assert!(flags.verbose);
        assert_eq!(path.as_deref(), Some("/etc/x.toml"));

        let missing = parse(["--config".to_string()], |arg, args| {
            value(arg, args).map(|_| true)
        });
        assert_eq!(
            missing,
            Err(Exit::Error("--config requires a value.".to_string()))
        );
    }
}
