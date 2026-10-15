//! The `process` check (M5-13, D-90): do the processes of a name run on the
//! probe's machine, how many, and what CPU and memory do they use together?
//!
//! For what isn't a service, or is several: a program started by a task, a
//! worker pool, a `java` among others told apart by its command line. The
//! probe only looks: it never starts or ends a process (D-59).
//!
//! It reads the list of processes the probe refreshes once a cycle
//! (`Monitor::refresh`), with what each used since the cycle before.

use sysinfo::System;

use common::checks::{Metric, metric};

use super::Outcome;
use super::files::matches;

const BYTES_PER_MB: f64 = 1024.0 * 1024.0;
/// Linux keeps 15 characters of a program's name; a longer one is cut.
const LINUX_NAME_CHARS: usize = 15;

/// A process, as the check looks at it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Seen {
    /// What it may be called by: its name as the system has it, its
    /// program file's name, and the first word of its command line.
    names: Vec<String>,
    /// Its command line, in one text.
    command: String,
    /// Percent of one processor it used since the last cycle.
    cpu: f32,
    /// Bytes of memory it holds.
    memory: u64,
}

/// A name without a path before it and `.exe` after it.
fn bare(name: &str) -> &str {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match name.len().checked_sub(4) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".exe") => {
            &name[..at]
        }
        _ => name,
    }
}

impl Seen {
    /// Is it the program `pattern` names, with `command` in its command
    /// line?
    fn is(&self, pattern: &str, command: &str) -> bool {
        let pattern = bare(pattern);
        let named = self.names.iter().map(|name| bare(name)).any(|name| {
            matches(pattern, name)
                // A long name that Linux cut: what is left starts the name
                // asked for.
                || (name.chars().count() == LINUX_NAME_CHARS
                    && pattern.chars().count() > LINUX_NAME_CHARS
                    && pattern.to_lowercase().starts_with(&name.to_lowercase()))
        });
        named
            && (command.is_empty()
                || self
                    .command
                    .to_lowercase()
                    .contains(&command.to_lowercase()))
    }
}

/// The machine's processes, as the check looks at them. Not their threads,
/// which Linux lists like processes.
fn seen(sys: &System) -> impl Iterator<Item = Seen> + '_ {
    sys.processes()
        .values()
        .filter(|process| process.thread_kind().is_none())
        .map(|process| {
            let text = |text: &std::ffi::OsStr| text.to_string_lossy().into_owned();
            let mut names = vec![text(process.name())];
            names.extend(process.exe().and_then(|exe| exe.file_name()).map(text));
            // Another user's program file can't be read without rights; its
            // command line can.
            names.extend(process.cmd().first().map(|first| text(first)));
            Seen {
                names,
                command: process
                    .cmd()
                    .iter()
                    .map(|word| word.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                cpu: process.cpu_usage(),
                memory: process.memory(),
            }
        })
}

/// How many of `processes` are the program asked for, against how many
/// should be. `cores`: the machine's processors, to say CPU as a share of
/// the whole machine.
fn judge(
    name: &str,
    command: &str,
    min: u32,
    max: Option<u32>,
    processes: impl Iterator<Item = Seen>,
    cores: usize,
) -> Outcome {
    let (mut count, mut cpu, mut memory) = (0u32, 0f64, 0u64);
    for process in processes.filter(|process| process.is(name, command)) {
        count += 1;
        cpu += f64::from(process.cpu);
        memory = memory.saturating_add(process.memory);
    }
    let cpu = (cpu / cores.max(1) as f64 * 10.0).round() / 10.0;
    let megabytes = (memory as f64 / BYTES_PER_MB).round();
    let what = if command.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({command})")
    };
    let metrics = vec![
        Metric::new(metric::PROCESSES, f64::from(count), ""),
        Metric::new(metric::CPU, cpu, metric::PERCENT),
        Metric::new(metric::MEMORY, megabytes, metric::MEGABYTES),
    ];
    let running = format!("{what}: {count} running");
    let outcome = if count < min {
        Outcome::down(match (count, min) {
            (0, 1) => format!("No process named {what} is running"),
            _ => format!("{running}, fewer than the {min} expected"),
        })
    } else if let Some(max) = max.filter(|max| count > *max) {
        Outcome::down(match max {
            0 => format!("{running}, and none should be"),
            _ => format!("{running}, more than the {max} allowed"),
        })
    } else if count == 0 {
        Outcome::up(format!("{what}: none running"))
    } else {
        Outcome::up(format!("{running}, {cpu}% CPU, {megabytes} MB"))
    };
    outcome.with_metrics(metrics)
}

pub fn check(name: &str, command: &str, min: u32, max: Option<u32>, sys: &System) -> Outcome {
    judge(name, command, min, max, seen(sys), sys.cpus().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    fn process(names: &[&str], command: &str, cpu: f32, megabytes: u64) -> Seen {
        Seen {
            names: names.iter().map(|name| name.to_string()).collect(),
            command: command.into(),
            cpu,
            memory: megabytes * 1024 * 1024,
        }
    }

    fn machine() -> Vec<Seen> {
        vec![
            process(&["nginx"], "nginx: master process /usr/sbin/nginx", 0.0, 10),
            process(&["nginx"], "nginx: worker process", 30.0, 40),
            process(&["nginx"], "nginx: worker process", 50.0, 50),
            process(
                &["java"],
                "/usr/bin/java -jar /opt/billing/app.jar",
                100.0,
                900,
            ),
            process(
                &["java"],
                "/usr/bin/java -jar /opt/search/app.jar",
                0.0,
                300,
            ),
            process(
                &[
                    "sqlservr.exe",
                    "sqlservr.exe",
                    "C:\\Program Files\\SQL\\sqlservr.exe",
                ],
                "\"C:\\Program Files\\SQL\\sqlservr.exe\" -sMSSQLSERVER",
                12.0,
                2048,
            ),
        ]
    }

    fn judged(name: &str, command: &str, min: u32, max: Option<u32>) -> Outcome {
        judge(name, command, min, max, machine().into_iter(), 4)
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value)
    }

    #[test]
    fn the_processes_of_a_name_are_counted_with_what_they_use_together() {
        let nginx = judged("nginx", "", 1, None);
        assert_eq!(nginx.status, Health::Up);
        // 80 % of one processor is 20 % of a machine with four.
        assert_eq!(nginx.message, "nginx: 3 running, 20% CPU, 100 MB");
        assert_eq!(value(&nginx, metric::PROCESSES), Some(3.0));
        assert_eq!(value(&nginx, metric::CPU), Some(20.0));
        assert_eq!(value(&nginx, metric::MEMORY), Some(100.0));

        // Told apart by the command line, whatever the case.
        let billing = judged("java", "BILLING/app.jar", 1, Some(1));
        assert_eq!(billing.status, Health::Up);
        assert_eq!(
            billing.message,
            "java (BILLING/app.jar): 1 running, 25% CPU, 900 MB"
        );
        assert_eq!(
            judged("java", "", 1, None).message,
            "java: 2 running, 25% CPU, 1200 MB"
        );
    }

    #[test]
    fn too_few_or_too_many_is_down_and_says_how_many_run() {
        let none = judged("backup-agent", "", 1, None);
        assert_eq!(none.status, Health::Down);
        assert_eq!(none.message, "No process named backup-agent is running");
        // The numbers are reported all the same.
        assert_eq!(value(&none, metric::PROCESSES), Some(0.0));
        assert_eq!(value(&none, metric::MEMORY), Some(0.0));

        let few = judged("nginx", "", 5, None);
        assert_eq!(few.status, Health::Down);
        assert_eq!(few.message, "nginx: 3 running, fewer than the 5 expected");
        let many = judged("nginx", "worker", 1, Some(1));
        assert_eq!(many.status, Health::Down);
        assert_eq!(
            many.message,
            "nginx (worker): 2 running, more than the 1 allowed"
        );
        // Exactly as many as asked for, at either end.
        assert_eq!(judged("nginx", "", 3, Some(3)).status, Health::Up);

        // A program that must not run.
        let unwanted = judged("java", "", 0, Some(0));
        assert_eq!(unwanted.status, Health::Down);
        assert_eq!(unwanted.message, "java: 2 running, and none should be");
        let absent = judged("teamviewer", "", 0, Some(0));
        assert_eq!(absent.status, Health::Up);
        assert_eq!(absent.message, "teamviewer: none running");
        let missing_text = judged("java", "nothing.jar", 1, None);
        assert_eq!(
            missing_text.message,
            "No process named java (nothing.jar) is running"
        );
    }

    #[test]
    fn a_name_is_matched_with_or_without_exe_whatever_its_case_and_by_pattern() {
        let sql = process(
            &["sqlservr.exe", "C:\\SQL\\Binn\\sqlservr.exe"],
            "sqlservr.exe -sMSSQLSERVER",
            0.0,
            1,
        );
        for name in [
            "sqlservr",
            "SQLSERVR.EXE",
            "sqlservr.exe",
            "sql*",
            "s?lservr",
        ] {
            assert!(sql.is(name, ""), "{name}");
        }
        for name in ["sql", "servr", "sqlservr.dll", "mysqld"] {
            assert!(!sql.is(name, ""), "{name}");
        }
        assert!(sql.is("sqlservr", "-smssqlserver"));
        assert!(!sql.is("sqlservr", "-sOTHER"));

        // Linux cut the name at 15 characters: the whole name still finds
        // it, by the program file, the command line, or what is left.
        let cut = process(&["systemd-journal"], "", 0.0, 1);
        assert!(cut.is("systemd-journald", ""));
        assert!(cut.is("systemd-journal", ""));
        assert!(!cut.is("systemd-logind", ""));
        // A name of 15 characters that is whole isn't the start of another.
        assert!(!process(&["exactly15chars_"], "", 0.0, 1).is("exactly15chars", ""));
        let by_command = process(
            &["NetworkManager-", "/usr/sbin/NetworkManager-dispatcher"],
            "",
            0.0,
            1,
        );
        assert!(by_command.is("networkmanager-dispatcher", ""));

        assert_eq!(bare("/usr/sbin/nginx"), "nginx");
        assert_eq!(bare("C:\\x\\Agent.EXE"), "Agent");
        assert_eq!(bare("exe"), "exe");
        assert_eq!(bare("é.exe"), "é");
    }

    #[test]
    fn this_machines_own_processes_are_found() {
        let mut sys = System::new_all();
        sys.refresh_all();
        // The test's own program, by the name the system gives it.
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().into_owned();
        let own = check(&name, "", 1, None, &sys);
        assert_eq!(own.status, Health::Up, "{}", own.message);
        assert!(value(&own, metric::PROCESSES) >= Some(1.0));
        assert!(value(&own, metric::MEMORY) > Some(0.0), "{}", own.message);

        let nobody = check("blentinel-no-such-program", "", 1, None, &sys);
        assert_eq!(nobody.status, Health::Down);
        assert_eq!(
            nobody.message,
            "No process named blentinel-no-such-program is running"
        );
        // Every process, counted once each: not its threads.
        let all = check("*", "", 1, None, &sys);
        assert_eq!(all.status, Health::Up);
        let processes = sys
            .processes()
            .values()
            .filter(|p| p.thread_kind().is_none())
            .count();
        assert_eq!(value(&all, metric::PROCESSES), Some(processes as f64));
    }

    /// By hand (`--ignored --nocapture`): `PROCESS_TEST_NAME` (and
    /// `PROCESS_TEST_COMMAND`) as this machine runs it, after two cycles so
    /// that its CPU is measured.
    #[tokio::test]
    #[ignore = "looks at this machine's processes: run by hand"]
    async fn a_named_process_of_this_machine() {
        let name = std::env::var("PROCESS_TEST_NAME").expect("PROCESS_TEST_NAME");
        let command = std::env::var("PROCESS_TEST_COMMAND").unwrap_or_default();
        let mut sys = System::new_all();
        sys.refresh_all();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        sys.refresh_all();
        let outcome = check(&name, &command, 1, None, &sys);
        println!("{:?}: {}", outcome.status, outcome.message);
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
    }
}
