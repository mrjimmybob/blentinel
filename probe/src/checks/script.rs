//! The `script` check (M5-17, D-100): a script of the probe's machine, run
//! and believed. The one exception to "checks are native Rust" (FP-2,
//! proposal 0006 #18), and off unless someone who administers the machine
//! turns it on there: the probe runs only files of the `scripts` folder in
//! its own folder, which only an administrator can make and fill. The hub
//! sends a file's name, never a command, a path or an argument, so a hub in
//! the wrong hands can run nothing but what the machine's administrator
//! chose to put there.
//!
//! The script speaks as a Nagios plugin does: its exit code is the verdict
//! (0 fine, 1 warning, 2 failed), its first line is the message, and what
//! follows a `|` on that line are its values (`time=0.4s size=12MB`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::checks::{Metric, SCRIPTS_FOLDER, metric};
use tokio::io::{AsyncRead, AsyncReadExt};

use super::Outcome;

/// How long a script may run. Then it is stopped.
const TIME_LIMIT: Duration = Duration::from_secs(20);

/// How much of what a script writes the probe keeps. The rest is thrown away.
const MAX_OUTPUT_BYTES: u64 = 64 * 1024;

/// The longest message taken from a script.
const MAX_MESSAGE_CHARS: usize = 300;

/// The most values taken from a script, and their longest name and unit.
const MAX_VALUES: usize = 20;
const MAX_KEY_CHARS: usize = 40;
const MAX_UNIT_CHARS: usize = 8;

/// Run the script called `name` of this probe's scripts folder.
pub async fn check(name: &str) -> Outcome {
    let folder = crate::config::get_base_dir().join(SCRIPTS_FOLDER);
    let file = match locate(&folder, name).and_then(|file| {
        platform::guard(&folder, &file)?;
        Ok(file)
    }) {
        Ok(file) => file,
        Err(why) => return Outcome::down(why),
    };
    run(name, &folder, &file, TIME_LIMIT).await
}

/// The file called `name` in `folder`. `Err`: why there is nothing to run,
/// in words; first of all, that scripts are off here.
fn locate(folder: &Path, name: &str) -> Result<PathBuf, String> {
    if !folder.is_dir() {
        return Err(format!(
            "Scripts are off on this probe. Who administers its machine turns them on by making the folder {} and putting {name} in it",
            folder.display()
        ));
    }
    let file = folder.join(name);
    match std::fs::symlink_metadata(&file) {
        Ok(found) if found.is_file() => Ok(file),
        // A link leads out of the folder, to a file others may change.
        Ok(found) if found.file_type().is_symlink() => Err(format!(
            "{name} is a link. The probe runs only files that are in {} themselves",
            folder.display()
        )),
        Ok(_) => Err(format!("{name} isn't a file")),
        Err(_) => Err(format!("No script named {name} in {}", folder.display())),
    }
}

/// Run `file` in `folder`, for no longer than `limit`, and say what it found.
async fn run(name: &str, folder: &Path, file: &Path, limit: Duration) -> Outcome {
    let mut command = match platform::command(name, file) {
        Ok(command) => command,
        Err(why) => return Outcome::down(why),
    };
    command
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Dropped at the time limit, which stops the script.
        .kill_on_drop(true);
    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return Outcome::down(platform::not_started(name, &e)),
    };
    let (out, err) = (child.stdout.take(), child.stderr.take());
    let ending = async { tokio::join!(kept(out), kept(err), child.wait()) };
    let outcome = match tokio::time::timeout(limit, ending).await {
        Ok((out, err, Ok(status))) => describe(
            name,
            &Ended {
                code: status.code(),
                out,
                err,
            },
        ),
        Ok((_, _, Err(e))) => Outcome::down(format!("{name} couldn't be waited for: {e}")),
        Err(_) => Outcome::down(format!(
            "{name} didn't end within {} s and was stopped",
            limit.as_secs()
        )),
    };
    outcome.with_latency(started.elapsed())
}

/// The first [`MAX_OUTPUT_BYTES`] a script writes to `pipe`. The rest is
/// read and dropped, so a script that writes a lot still ends.
async fn kept(pipe: Option<impl AsyncRead + Unpin>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = (&mut pipe)
        .take(MAX_OUTPUT_BYTES)
        .read_to_end(&mut bytes)
        .await;
    let _ = tokio::io::copy(&mut pipe, &mut tokio::io::sink()).await;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// How a script ended: its exit code (`None`: the system stopped it) and
/// what it wrote.
#[derive(Debug, Default)]
struct Ended {
    code: Option<i32>,
    out: String,
    err: String,
}

/// What a script's ending is worth, as a Nagios plugin's: 0 is fine, 1 a
/// warning (the hub shows it as one), 2 a failure. Any other code is no
/// verdict, so the check is Down and says the code.
fn describe(name: &str, ended: &Ended) -> Outcome {
    let (mut said, mut values) = heard(first_line(&ended.out));
    if said.is_empty() {
        said = shortened(first_line(&ended.err));
    }
    let or = |silent: &str| {
        if said.is_empty() {
            format!("{name} {silent} and said nothing")
        } else {
            said.clone()
        }
    };
    match ended.code {
        Some(0) => Outcome::up(or("ended well")).with_metrics(values),
        Some(1) => {
            values.push(Metric::new(metric::WARNINGS, 1.0, ""));
            Outcome::up(or("ended with a warning (code 1)")).with_metrics(values)
        }
        Some(2) => Outcome::down(or("failed (code 2)")).with_metrics(values),
        Some(code) => {
            let said = if said.is_empty() {
                String::new()
            } else {
                format!(": {said}")
            };
            Outcome::down(format!(
                "{name} ended with code {code}, which is no verdict (0: fine, 1: warning, 2: failed){said}"
            ))
        }
        None => Outcome::down(format!("{name} was stopped by the system before it ended")),
    }
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// `text` without control characters, no longer than a message may be.
fn shortened(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.chars().count() <= MAX_MESSAGE_CHARS {
        return clean.to_string();
    }
    let mut short: String = clean.chars().take(MAX_MESSAGE_CHARS - 1).collect();
    short.push('…');
    short
}

/// A script's line: its message, and the values after the `|`.
fn heard(line: &str) -> (String, Vec<Metric>) {
    let (words, values) = line.split_once('|').unwrap_or((line, ""));
    let mut metrics: Vec<Metric> = Vec::new();
    for value in labelled(values).iter().filter_map(|v| measured(v)) {
        // `warnings` is the probe's own word for a script's exit code.
        let taken = value.key == metric::WARNINGS || metrics.iter().any(|m| m.key == value.key);
        if !taken && metrics.len() < MAX_VALUES {
            metrics.push(value);
        }
    }
    (shortened(words), metrics)
}

/// The values of a line, apart: `time=0.4s 'disk c'=12MB` is two. A label
/// in single quotes may have spaces.
fn labelled(values: &str) -> Vec<String> {
    let mut all = Vec::new();
    let mut one = String::new();
    let mut quoted = false;
    for c in values.chars() {
        match c {
            '\'' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !one.is_empty() {
                    all.push(std::mem::take(&mut one));
                }
            }
            c => one.push(c),
        }
    }
    if !one.is_empty() {
        all.push(one);
    }
    all
}

/// One value: `label=number[unit]`, and after a `;` the plugin's own limits,
/// which the probe doesn't use. `None`: not a number (`U` is "unknown").
fn measured(value: &str) -> Option<Metric> {
    let (label, rest) = value.rsplit_once('=')?;
    let label: String = label.chars().filter(|c| !c.is_control()).collect();
    let label = label.trim();
    let reading = rest.split(';').next().unwrap_or("");
    let digits = reading
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .unwrap_or(reading.len());
    let number: f64 = reading[..digits].parse().ok()?;
    let unit = &reading[digits..];
    let fits = !label.is_empty()
        && label.chars().count() <= MAX_KEY_CHARS
        && unit.chars().count() <= MAX_UNIT_CHARS
        && unit
            .chars()
            .all(|c| c.is_alphabetic() || c == '%' || c == '/')
        && number.is_finite();
    fits.then(|| Metric::new(label, number, unit))
}

#[cfg(unix)]
mod platform {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tokio::process::Command;

    /// Who may write: a script anyone else can change is anyone's command,
    /// run as the probe.
    const OTHERS_WRITE: u32 = 0o022;
    const ANYONE_RUNS: u32 = 0o111;

    /// `Err`: why the probe won't run `file`, in words.
    pub(super) fn guard(folder: &Path, file: &Path) -> Result<(), String> {
        let mode = |path: &Path| {
            std::fs::metadata(path)
                .map(|m| m.permissions().mode())
                .map_err(|e| format!("Can't read {}: {e}", path.display()))
        };
        if mode(folder)? & OTHERS_WRITE != 0 {
            return Err(format!(
                "Others than its owner can write to {}, so the probe runs nothing from it (chmod go-w)",
                folder.display()
            ));
        }
        let script = mode(file)?;
        if script & OTHERS_WRITE != 0 {
            return Err(format!(
                "Others than its owner can change {}, so the probe doesn't run it (chmod go-w)",
                file.display()
            ));
        }
        if script & ANYONE_RUNS == 0 {
            return Err(format!(
                "{} isn't marked as a program (chmod +x)",
                file.display()
            ));
        }
        Ok(())
    }

    /// The script is run as it is: its first line says what runs it.
    pub(super) fn command(_name: &str, file: &Path) -> Result<Command, String> {
        Ok(Command::new(file))
    }

    pub(super) fn not_started(name: &str, error: &std::io::Error) -> String {
        /// `ENOEXEC`: neither a program nor a file that names its interpreter.
        const NOT_A_PROGRAM: i32 = 8;
        match error.raw_os_error() {
            Some(NOT_A_PROGRAM) => {
                format!("{name} doesn't say what runs it: its first line must, e.g. #!/bin/sh")
            }
            _ if error.kind() == std::io::ErrorKind::NotFound => {
                format!("{name} can't be started: what its first line names isn't on this machine")
            }
            _ if error.kind() == std::io::ErrorKind::PermissionDenied => {
                format!("The probe isn't allowed to run {name}")
            }
            _ => format!("{name} can't be started: {error}"),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use common::service::windows as win;
    use tokio::process::Command;

    /// No console window for the script (the service has none).
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// `Err`: why the probe won't run `file`, in words. The probe runs as
    /// SYSTEM, so a script someone else can change would be theirs to run
    /// as SYSTEM: the folder and the file must be SYSTEM's and the
    /// Administrators' alone, as the installed probe's folder is.
    pub(super) fn guard(folder: &Path, file: &Path) -> Result<(), String> {
        for path in [folder, file] {
            let sddl = win::dacl_sddl(path)
                .map_err(|e| format!("Can't read who may change {}: {e}", path.display()))?;
            if !win::only_system_and_admins(&sddl) {
                return Err(format!(
                    "{} is open to others than SYSTEM and Administrators, so the probe runs nothing from it",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    /// Windows runs a file by its ending. PowerShell and `cmd` are part of
    /// every Windows, and are started from the system's own folder.
    pub(super) fn command(name: &str, file: &Path) -> Result<Command, String> {
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or("C:\\Windows".into()))
            .join("System32");
        let ending = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let mut command = match ending.as_str() {
            "ps1" => {
                let mut c = Command::new(system.join("WindowsPowerShell\\v1.0\\powershell.exe"));
                c.args(["-NoLogo", "-NoProfile", "-NonInteractive"])
                    .args(["-ExecutionPolicy", "Bypass", "-File"])
                    .arg(file);
                c
            }
            "cmd" | "bat" => {
                let mut c = Command::new(system.join("cmd.exe"));
                c.args(["/D", "/C"]).arg(file);
                c
            }
            "exe" => Command::new(file),
            _ => {
                return Err(format!(
                    "{name} isn't something Windows runs: a script here ends in .ps1, .cmd, .bat or .exe"
                ));
            }
        };
        command.creation_flags(CREATE_NO_WINDOW);
        Ok(command)
    }

    pub(super) fn not_started(name: &str, error: &std::io::Error) -> String {
        format!("{name} can't be started: {error}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    /// The deadline of the tests about what a script's ending means, not
    /// about time: only a script that hangs reaches it. With the probe's
    /// own [`TIME_LIMIT`] they failed for the wrong reason when a shared CI
    /// runner stalled: on 2026-10-20 a Windows runner took over 20 s to
    /// start PowerShell, which takes 0.2 to 2 s, and other tests in the run
    /// stalled with it. The time limit has tests of its own, with a limit
    /// that is always reached.
    const NO_HURRY: Duration = Duration::from_secs(120);

    fn ended(code: i32, out: &str) -> Ended {
        Ended {
            code: Some(code),
            out: out.into(),
            err: String::new(),
        }
    }

    fn pairs(outcome: &Outcome) -> Vec<(&str, f64, &str)> {
        outcome
            .metrics
            .iter()
            .map(|m| (m.key.as_str(), m.value, m.unit.as_str()))
            .collect()
    }

    #[test]
    fn the_exit_code_is_the_verdict_and_the_first_line_the_message() {
        let fine = describe("backup.sh", &ended(0, "\nBackup OK, 3 hours old\nmore\n"));
        assert_eq!(fine.status, Health::Up);
        assert_eq!(fine.message, "Backup OK, 3 hours old");
        assert!(fine.metrics.is_empty());

        // A warning is Up with the sign the hub turns into a Warning.
        let warning = describe("backup.sh", &ended(1, "Backup is 30 hours old"));
        assert_eq!(warning.status, Health::Up);
        assert_eq!(warning.message, "Backup is 30 hours old");
        assert_eq!(pairs(&warning), [("warnings", 1.0, "")]);

        let failed = describe("backup.sh", &ended(2, "No backup since Monday"));
        assert_eq!(failed.status, Health::Down);
        assert_eq!(failed.message, "No backup since Monday");
    }

    #[test]
    fn any_other_code_is_no_verdict() {
        let unknown = describe("backup.sh", &ended(3, "UNKNOWN: no such host"));
        assert_eq!(unknown.status, Health::Down);
        assert_eq!(
            unknown.message,
            "backup.sh ended with code 3, which is no verdict (0: fine, 1: warning, 2: failed): UNKNOWN: no such host"
        );
        let shell = describe("backup.sh", &ended(127, ""));
        assert!(shell.message.starts_with("backup.sh ended with code 127"));
        assert!(shell.message.ends_with("failed)"), "{}", shell.message);

        let stopped = describe("backup.sh", &Ended::default());
        assert_eq!(stopped.status, Health::Down);
        assert_eq!(
            stopped.message,
            "backup.sh was stopped by the system before it ended"
        );
    }

    #[test]
    fn a_silent_script_is_spoken_for_and_its_errors_are_heard() {
        assert_eq!(
            describe("x.sh", &ended(0, "")).message,
            "x.sh ended well and said nothing"
        );
        assert_eq!(
            describe("x.sh", &ended(1, " \n")).message,
            "x.sh ended with a warning (code 1) and said nothing"
        );
        assert_eq!(
            describe("x.sh", &ended(2, "")).message,
            "x.sh failed (code 2) and said nothing"
        );
        let complained = Ended {
            code: Some(2),
            out: String::new(),
            err: "\nx.sh: 3: curl: not found\n".into(),
        };
        assert_eq!(
            describe("x.sh", &complained).message,
            "x.sh: 3: curl: not found"
        );
    }

    #[test]
    fn values_follow_the_bar_as_a_nagios_plugins_do() {
        let outcome = describe(
            "raid.sh",
            &ended(
                0,
                "RAID OK | time=0.42s 'disk c'=12.5MB;80;90;0;100 used=45% temp=-3 load=1,5 n=U bad",
            ),
        );
        assert_eq!(outcome.message, "RAID OK");
        assert_eq!(
            pairs(&outcome),
            [
                ("time", 0.42, "s"),
                ("disk c", 12.5, "MB"),
                ("used", 45.0, "%"),
                ("temp", -3.0, ""),
            ]
        );
        // A warning's sign comes after the script's own values.
        let warning = describe("raid.sh", &ended(1, "Degraded | disks=3"));
        assert_eq!(pairs(&warning), [("disks", 3.0, ""), ("warnings", 1.0, "")]);
    }

    #[test]
    fn a_script_cant_fake_the_probes_sign_or_flood_the_hub() {
        // `warnings` is the probe's word: a script's own isn't taken.
        let faked = describe("x.sh", &ended(0, "fine | warnings=5 a=1 a=2"));
        assert_eq!(pairs(&faked), [("a", 1.0, "")]);

        let many: String = (0..50).map(|n| format!("v{n}={n} ")).collect();
        let flood = describe("x.sh", &ended(0, &format!("ok | {many}")));
        assert_eq!(flood.metrics.len(), MAX_VALUES);

        let long = describe("x.sh", &ended(0, &"é".repeat(1000)));
        assert_eq!(long.message.chars().count(), MAX_MESSAGE_CHARS);
        assert!(long.message.ends_with('…'));

        let odd = describe(
            "x.sh",
            &ended(0, "a\u{1b}[31mred\tb | k\u{7}=1 x=1e9 y=nan z=1toolongunit"),
        );
        assert_eq!(odd.message, "a[31mredb");
        assert_eq!(pairs(&odd), [("k", 1.0, "")]);
        let long_key = format!("ok | {}=1", "k".repeat(MAX_KEY_CHARS + 1));
        assert!(describe("x.sh", &ended(0, &long_key)).metrics.is_empty());
    }

    #[test]
    fn scripts_are_off_until_the_folder_is_made() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(SCRIPTS_FOLDER);
        let off = locate(&folder, "backup.sh").unwrap_err();
        assert!(off.starts_with("Scripts are off on this probe."), "{off}");
        assert!(off.contains("backup.sh"), "{off}");

        std::fs::create_dir(&folder).unwrap();
        let none = locate(&folder, "backup.sh").unwrap_err();
        assert!(none.starts_with("No script named backup.sh in "), "{none}");

        std::fs::create_dir(folder.join("a_folder")).unwrap();
        assert_eq!(
            locate(&folder, "a_folder").unwrap_err(),
            "a_folder isn't a file"
        );

        std::fs::write(folder.join("backup.sh"), "#!/bin/sh\n").unwrap();
        assert_eq!(
            locate(&folder, "backup.sh").unwrap(),
            folder.join("backup.sh")
        );
    }

    #[cfg(unix)]
    mod on_unix {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        use super::super::*;
        use common::checks::Health;

        fn script(folder: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
            let file = folder.join(name);
            std::fs::write(&file, body).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
            file
        }

        fn folder() -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            dir
        }

        #[test]
        fn a_link_out_of_the_folder_isnt_run() {
            let dir = folder();
            std::os::unix::fs::symlink("/bin/sh", dir.path().join("sh")).unwrap();
            let why = locate(dir.path(), "sh").unwrap_err();
            assert!(why.starts_with("sh is a link."), "{why}");
        }

        #[test]
        fn what_others_can_change_isnt_run() {
            let dir = folder();
            let good = script(dir.path(), "good.sh", "#!/bin/sh\n", 0o755);
            assert_eq!(platform::guard(dir.path(), &good), Ok(()));

            let open = script(dir.path(), "open.sh", "#!/bin/sh\n", 0o777);
            let why = platform::guard(dir.path(), &open).unwrap_err();
            assert!(
                why.starts_with("Others than its owner can change "),
                "{why}"
            );

            let plain = script(dir.path(), "plain.sh", "#!/bin/sh\n", 0o644);
            let why = platform::guard(dir.path(), &plain).unwrap_err();
            assert!(
                why.ends_with("isn't marked as a program (chmod +x)"),
                "{why}"
            );

            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
            let why = platform::guard(dir.path(), &good).unwrap_err();
            assert!(
                why.starts_with("Others than its owner can write to "),
                "{why}"
            );
        }

        #[tokio::test]
        async fn a_script_is_run_and_believed() {
            let dir = folder();
            let body = "#!/bin/sh\necho \"in $(basename \"$PWD\") | n=7\"\nexit 1\n";
            let file = script(dir.path(), "warn.sh", body, 0o755);
            let outcome = run("warn.sh", dir.path(), &file, super::NO_HURRY).await;
            assert_eq!(outcome.status, Health::Up);
            let here = dir.path().file_name().unwrap().to_string_lossy();
            assert_eq!(outcome.message, format!("in {here}"));
            assert_eq!(outcome.metrics.len(), 2);
            assert!(outcome.latency_ms.is_some());

            let file = script(
                dir.path(),
                "down.sh",
                "#!/bin/sh\necho gone >&2\nexit 2\n",
                0o755,
            );
            let outcome = run("down.sh", dir.path(), &file, super::NO_HURRY).await;
            assert_eq!(
                (outcome.status, outcome.message.as_str()),
                (Health::Down, "gone")
            );
        }

        #[tokio::test]
        async fn a_file_that_isnt_a_script_says_so() {
            let dir = folder();
            let file = script(dir.path(), "text", "just words\n", 0o755);
            let outcome = run("text", dir.path(), &file, super::NO_HURRY).await;
            assert_eq!(outcome.status, Health::Down);
            assert!(
                outcome.message.contains("first line"),
                "{}",
                outcome.message
            );

            let file = script(dir.path(), "lost.sh", "#!/no/such/shell\n", 0o755);
            let outcome = run("lost.sh", dir.path(), &file, super::NO_HURRY).await;
            assert!(
                outcome.message.contains("isn't on this machine"),
                "{}",
                outcome.message
            );
        }

        #[tokio::test]
        async fn a_script_that_writes_without_end_still_ends() {
            let dir = folder();
            let body = "#!/bin/sh\necho first\nhead -c 2000000 /dev/zero\nexit 0\n";
            let file = script(dir.path(), "loud.sh", body, 0o755);
            let outcome = run("loud.sh", dir.path(), &file, super::NO_HURRY).await;
            assert_eq!(
                (outcome.status, outcome.message.as_str()),
                (Health::Up, "first")
            );
        }

        #[tokio::test]
        async fn a_script_that_doesnt_end_is_stopped() {
            let dir = folder();
            let file = script(dir.path(), "slow.sh", "#!/bin/sh\nexec sleep 600\n", 0o755);
            let outcome = run("slow.sh", dir.path(), &file, Duration::from_secs(1)).await;
            assert_eq!(outcome.status, Health::Down);
            assert_eq!(
                outcome.message,
                "slow.sh didn't end within 1 s and was stopped"
            );
        }
    }

    #[cfg(windows)]
    mod on_windows {
        use super::super::*;
        use common::checks::Health;

        #[tokio::test]
        async fn powershell_and_cmd_scripts_are_run_by_their_ending() {
            let dir = tempfile::tempdir().unwrap();
            let ps = dir.path().join("warn.ps1");
            std::fs::write(&ps, "Write-Output \"from powershell | n=7\"\nexit 1\n").unwrap();
            let outcome = run("warn.ps1", dir.path(), &ps, super::NO_HURRY).await;
            assert_eq!(outcome.message, "from powershell");
            assert_eq!(outcome.status, Health::Up);
            assert_eq!(outcome.metrics.len(), 2);

            let cmd = dir.path().join("down.cmd");
            std::fs::write(&cmd, "@echo off\r\necho from cmd\r\nexit /b 2\r\n").unwrap();
            let outcome = run("down.cmd", dir.path(), &cmd, super::NO_HURRY).await;
            assert_eq!(
                (outcome.status, outcome.message.as_str()),
                (Health::Down, "from cmd")
            );
        }

        /// The time limit on Windows, where the script is PowerShell's: it
        /// is always reached, however fast PowerShell starts, and the script
        /// is stopped, not left running (it would write `ran-on`).
        #[tokio::test]
        async fn a_powershell_script_that_doesnt_end_is_stopped() {
            let dir = tempfile::tempdir().unwrap();
            let ps = dir.path().join("slow.ps1");
            let body = "Start-Sleep -Milliseconds 1500\nSet-Content -Path ran-on -Value late\nStart-Sleep 600\n";
            std::fs::write(&ps, body).unwrap();
            let outcome = run("slow.ps1", dir.path(), &ps, Duration::from_secs(1)).await;
            assert_eq!(outcome.status, Health::Down);
            assert_eq!(
                outcome.message,
                "slow.ps1 didn't end within 1 s and was stopped"
            );
            tokio::time::sleep(Duration::from_secs(4)).await;
            assert!(!dir.path().join("ran-on").exists(), "slow.ps1 kept running");
        }

        #[test]
        fn a_file_windows_doesnt_run_says_so() {
            let dir = tempfile::tempdir().unwrap();
            for name in ["check.sh", "check", "check.py"] {
                let why = platform::command(name, &dir.path().join(name))
                    .err()
                    .unwrap();
                assert!(why.contains(".ps1, .cmd, .bat or .exe"), "{why}");
            }
        }

        #[test]
        fn only_a_folder_of_system_and_administrators_is_run_from() {
            // A temporary folder is its user's, not SYSTEM's and the
            // Administrators' alone.
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("check.cmd");
            std::fs::write(&file, "@exit /b 0\r\n").unwrap();
            let why = platform::guard(dir.path(), &file).unwrap_err();
            // Made SYSTEM's and the Administrators' alone, as `probe
            // install` makes the probe's folder, it is run from. Its owner
            // may still read and set who has access, so this needs no rights.
            use common::service::windows as win;
            win::restrict_to_admins(dir.path()).unwrap();
            let verdict = platform::guard(dir.path(), &file);
            // Opened again, so the temporary folder can be removed.
            win::apply_sddl(dir.path(), "D:(A;OICI;FA;;;WD)").unwrap();
            assert_eq!(verdict, Ok(()));
            assert!(
                why.contains("open to others than SYSTEM and Administrators"),
                "{why}"
            );
        }
    }
}
