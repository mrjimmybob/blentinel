//! The `hyperv` check (M5-16f, D-98): do the virtual machines of this
//! Hyper-V host run, by what Hyper-V itself says of them?
//!
//! The probe runs on the host and reads Hyper-V's own list from Windows'
//! management store, as `Get-VM` does: no account, no network, nothing set
//! up on the host. The installed probe may read it (it runs as the system);
//! an ordinary user is shown no machine at all.
//!
//! Down when a machine the check names doesn't run, or when Hyper-V says a
//! machine is in a critical state. A machine with a failure that isn't
//! critical is counted for the hub, which makes a Warning of it (the probe
//! says Up or Down only).

// On another system the check only says it can't run there: what reads
// and judges the machines is built for Windows, and for the tests.
#![cfg_attr(not(any(windows, test)), allow(dead_code))]

use common::checks::{Metric, metric};

use super::Outcome;
use super::files::matches;

/// How many machines a message names before it says "and N more".
const NAMED_AT_MOST: usize = 5;

/// What a machine is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Power {
    Running,
    Off,
    Saved,
    Paused,
    Starting,
    Stopping,
    /// A state Hyper-V gives by a number the probe doesn't know.
    Other(u16),
}

impl Power {
    /// From Hyper-V's `EnabledState`: the standard numbers and its own.
    fn of(state: u16) -> Self {
        match state {
            2 => Power::Running,
            3 => Power::Off,
            6 | 32769 => Power::Saved,
            9 | 32768 => Power::Paused,
            // Starting, resuming.
            10 | 32770 | 32777 => Power::Starting,
            // Shutting down, saving, stopping, pausing.
            4 | 32773 | 32774 | 32776 => Power::Stopping,
            other => Power::Other(other),
        }
    }

    fn said(self) -> String {
        match self {
            Power::Running => "is running".to_string(),
            Power::Off => "is off".to_string(),
            Power::Saved => "is saved, not running".to_string(),
            Power::Paused => "is paused".to_string(),
            Power::Starting => "is starting".to_string(),
            Power::Stopping => "is stopping".to_string(),
            Power::Other(state) => format!("is in state {state}, not running"),
        }
    }
}

/// How Hyper-V says a machine is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Condition {
    Fine,
    /// A major failure: it works, with trouble.
    Failure,
    /// A critical failure: it can't go on (its disk is full or gone).
    Critical,
}

impl Condition {
    /// From Hyper-V's `HealthState`: 5 OK, 20 major, 25 critical failure.
    fn of(health: Option<u16>) -> Self {
        match health {
            Some(25) => Condition::Critical,
            Some(20) => Condition::Failure,
            _ => Condition::Fine,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Vm {
    name: String,
    power: Power,
    condition: Condition,
}

fn machines(count: usize) -> String {
    match count {
        1 => "1 virtual machine".to_string(),
        count => format!("{count} virtual machines"),
    }
}

/// The names, the first few of many.
fn listed(names: &[&str]) -> String {
    let mut shown = names
        .iter()
        .take(NAMED_AT_MOST)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > NAMED_AT_MOST {
        shown.push_str(&format!(" and {} more", names.len() - NAMED_AT_MOST));
    }
    shown
}

/// What the check says of the host's machines `vms`, of which those
/// `wanted` names must run (none: every one counts, none must run).
fn describe(vms: &[Vm], wanted: &[String]) -> Outcome {
    let counted: Vec<&Vm> = vms
        .iter()
        .filter(|vm| wanted.is_empty() || wanted.iter().any(|name| matches(name, &vm.name)))
        .collect();
    let running = counted
        .iter()
        .filter(|vm| vm.power == Power::Running)
        .count();

    let mut down = Vec::new();
    // A name no machine has: it was renamed or removed, and nobody watches
    // what the check was made for.
    for name in wanted {
        if !vms.iter().any(|vm| matches(name, &vm.name)) {
            down.push(format!("No virtual machine here is named {name}"));
        }
    }
    for vm in &counted {
        if vm.condition == Condition::Critical {
            down.push(format!(
                "{}: Hyper-V says it is in a critical state",
                vm.name
            ));
        } else if !wanted.is_empty() && vm.power != Power::Running {
            down.push(format!("{} {}", vm.name, vm.power.said()));
        }
    }
    let warned: Vec<String> = counted
        .iter()
        .filter(|vm| vm.condition == Condition::Failure)
        .map(|vm| format!("{}: Hyper-V says it has a failure", vm.name))
        .collect();

    let metrics = vec![
        Metric::new(metric::RUNNING, running as f64, ""),
        Metric::new(metric::VMS, counted.len() as f64, ""),
        Metric::new(metric::WARNINGS, warned.len() as f64, ""),
    ];
    let summary = if counted.is_empty() {
        match wanted.is_empty() {
            true => "Hyper-V has no virtual machine on this host".to_string(),
            false => String::new(),
        }
    } else if running == counted.len() {
        match running {
            1 => "1 virtual machine running".to_string(),
            all => format!("All {all} virtual machines running"),
        }
    } else {
        let mut summary = format!("{running} of {} running", machines(counted.len()));
        // Where they must run, each one that doesn't is said above.
        if wanted.is_empty() {
            let idle: Vec<&str> = counted
                .iter()
                .filter(|vm| vm.power != Power::Running)
                .map(|vm| vm.name.as_str())
                .collect();
            summary.push_str(&format!(" (not running: {})", listed(&idle)));
        }
        summary
    };

    let is_down = !down.is_empty();
    let mut said = down;
    said.extend(warned);
    said.extend(Some(summary).filter(|summary| !summary.is_empty()));
    let message = said.join("; ");
    let outcome = if is_down {
        Outcome::down(message)
    } else {
        Outcome::up(message)
    };
    outcome.with_metrics(metrics)
}

/// The host's machines from Hyper-V's rows: `Msvm_ComputerSystem` lists the
/// host too, under its computer name; a machine's `Name` is its ID.
fn from_rows(rows: &[(String, String, Option<u16>, Option<u16>)]) -> Vec<Vm> {
    let is_id =
        |name: &str| name.len() == 36 && name.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    let mut vms: Vec<Vm> = rows
        .iter()
        .filter(|(id, ..)| is_id(id))
        .map(|(id, name, state, health)| Vm {
            name: match name.trim() {
                "" => id.clone(),
                name => name.to_string(),
            },
            power: Power::of(state.unwrap_or(0)),
            condition: Condition::of(*health),
        })
        .collect();
    vms.sort_by_key(|vm| vm.name.to_lowercase());
    vms
}

pub async fn check(wanted: &[String]) -> Outcome {
    match platform::machines().await {
        Ok(vms) => describe(&vms, wanted),
        Err(why) => Outcome::down(why),
    }
}

#[cfg(windows)]
mod platform {
    use super::{Vm, from_rows};
    use crate::checks::wmi::{self, NO_SUCH_NAMESPACE, Namespace};

    /// Where Hyper-V keeps what it knows (Windows 8 and Server 2012 on):
    /// what `Get-VM` shows.
    const NAMESPACE: &str = "ROOT\\virtualization\\v2";
    const FIELDS: [&str; 4] = ["Name", "ElementName", "EnabledState", "HealthState"];

    pub async fn machines() -> Result<Vec<Vm>, String> {
        let vms = wmi::read(|| {
            ask().map_err(|e| match e.code() {
                NO_SUCH_NAMESPACE => {
                    "This machine isn't a Hyper-V host (Hyper-V isn't turned on)".to_string()
                }
                _ => format!("Hyper-V can't be read: {}", e.message()),
            })
        })
        .await
        .unwrap_or_else(|| Err("Reading Hyper-V failed".to_string()))?;
        // Hyper-V answers an ordinary user with an empty list, not with a
        // refusal: "no machine" would be said of a host full of them.
        if vms.is_empty() && !common::service::windows::is_admin() {
            return Err(
                "Hyper-V shows its machines only to the installed probe or an administrator, \
                 and this probe runs as an ordinary user"
                    .to_string(),
            );
        }
        Ok(vms)
    }

    fn ask() -> windows::core::Result<Vec<Vm>> {
        let hyperv = Namespace::open(NAMESPACE)?;
        let rows = hyperv.rows("Msvm_ComputerSystem", &FIELDS)?;
        let text = |row: &wmi::Row, field| row.get(field).cloned().unwrap_or_default();
        let number = |row: &wmi::Row, field| row.get(field).and_then(|n| n.parse::<u16>().ok());
        let rows: Vec<_> = rows
            .iter()
            .map(|row| {
                (
                    text(row, "Name"),
                    text(row, "ElementName"),
                    number(row, "EnabledState"),
                    number(row, "HealthState"),
                )
            })
            .collect();
        Ok(from_rows(&rows))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Vm;

    pub async fn machines() -> Result<Vec<Vm>, String> {
        Err("A Hyper-V check works on Windows only: on the Hyper-V host itself".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    fn vm(name: &str, power: Power) -> Vm {
        Vm {
            name: name.into(),
            power,
            condition: Condition::Fine,
        }
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value)
    }

    fn names(wanted: &[&str]) -> Vec<String> {
        wanted.iter().map(|name| name.to_string()).collect()
    }

    fn host() -> Vec<Vm> {
        vec![
            vm("dc01", Power::Running),
            vm("sql-prod", Power::Running),
            vm("sql-test", Power::Off),
            vm("web01", Power::Saved),
        ]
    }

    #[test]
    fn without_names_every_machine_is_counted_and_none_must_run() {
        let all = describe(&host(), &[]);
        assert_eq!(all.status, Health::Up);
        assert_eq!(
            all.message,
            "2 of 4 virtual machines running (not running: sql-test, web01)"
        );
        let keys: Vec<&str> = all.metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, [metric::RUNNING, metric::VMS, metric::WARNINGS]);
        assert_eq!(value(&all, metric::RUNNING), Some(2.0));
        assert_eq!(value(&all, metric::VMS), Some(4.0));
        assert_eq!(value(&all, metric::WARNINGS), Some(0.0));

        let running = describe(&host()[..2], &[]);
        assert_eq!(running.message, "All 2 virtual machines running");
        let one = describe(&host()[..1], &[]);
        assert_eq!(one.message, "1 virtual machine running");
        let off = describe(&host()[2..3], &[]);
        assert_eq!(
            off.message,
            "0 of 1 virtual machine running (not running: sql-test)"
        );

        // A host without machines is no trouble of its own.
        let none = describe(&[], &[]);
        assert_eq!(none.status, Health::Up);
        assert_eq!(none.message, "Hyper-V has no virtual machine on this host");
        assert_eq!(value(&none, metric::VMS), Some(0.0));

        // Of many that don't run, the first few are named.
        let many: Vec<Vm> = (1..=8)
            .map(|n| vm(&format!("lab{n}"), Power::Off))
            .collect();
        assert_eq!(
            describe(&many, &[]).message,
            "0 of 8 virtual machines running (not running: lab1, lab2, lab3, lab4, lab5 and 3 \
             more)"
        );
    }

    #[test]
    fn a_named_machine_must_run() {
        let fine = describe(&host(), &names(&["dc01", "SQL-PROD"]));
        assert_eq!(fine.status, Health::Up);
        assert_eq!(fine.message, "All 2 virtual machines running");
        assert_eq!(value(&fine, metric::VMS), Some(2.0));

        // A pattern covers every machine it matches.
        let both = describe(&host(), &names(&["sql-*"]));
        assert_eq!(both.status, Health::Down);
        assert_eq!(
            both.message,
            "sql-test is off; 1 of 2 virtual machines running"
        );
        assert_eq!(value(&both, metric::RUNNING), Some(1.0));

        let saved = describe(&host(), &names(&["web01"]));
        assert_eq!(saved.status, Health::Down);
        assert_eq!(
            saved.message,
            "web01 is saved, not running; 0 of 1 virtual machine running"
        );

        // A name no machine has is trouble too: nobody watches it.
        let gone = describe(&host(), &names(&["dc01", "mail01"]));
        assert_eq!(gone.status, Health::Down);
        assert_eq!(
            gone.message,
            "No virtual machine here is named mail01; 1 virtual machine running"
        );
        let nothing = describe(&host(), &names(&["mail*"]));
        assert_eq!(nothing.status, Health::Down);
        assert_eq!(nothing.message, "No virtual machine here is named mail*");
        assert_eq!(value(&nothing, metric::VMS), Some(0.0));

        for (power, said) in [
            (Power::Paused, "x is paused"),
            (Power::Starting, "x is starting"),
            (Power::Stopping, "x is stopping"),
            (Power::Other(7), "x is in state 7, not running"),
        ] {
            let outcome = describe(&[vm("x", power)], &names(&["x"]));
            assert_eq!(outcome.status, Health::Down);
            assert!(outcome.message.starts_with(said), "{}", outcome.message);
        }
    }

    #[test]
    fn a_critical_machine_is_down_and_a_failure_is_counted_for_the_hub() {
        let mut machines = host();
        machines[1].condition = Condition::Failure;
        let warned = describe(&machines, &[]);
        assert_eq!(warned.status, Health::Up);
        assert_eq!(
            warned.message,
            "sql-prod: Hyper-V says it has a failure; 2 of 4 virtual machines running (not \
             running: sql-test, web01)"
        );
        assert_eq!(value(&warned, metric::WARNINGS), Some(1.0));

        // Critical is Down, named or not; the worst is said first.
        machines[0].condition = Condition::Critical;
        let critical = describe(&machines, &[]);
        assert_eq!(critical.status, Health::Down);
        assert!(
            critical.message.starts_with(
                "dc01: Hyper-V says it is in a critical state; sql-prod: Hyper-V says it has a \
                 failure; "
            ),
            "{}",
            critical.message
        );
        // A machine the check doesn't name doesn't count.
        let others = describe(&machines, &names(&["web01"]));
        assert_eq!(
            others.message,
            "web01 is saved, not running; 0 of 1 virtual machine running"
        );
        assert_eq!(value(&others, metric::WARNINGS), Some(0.0));
    }

    #[test]
    fn machines_are_read_as_hyperv_lists_them() {
        let row =
            |id: &str, name: &str, state, health| (id.to_string(), name.to_string(), state, health);
        let vms = from_rows(&[
            // The host itself, under its computer name.
            row("WOPR", "WOPR", Some(2), Some(5)),
            row(
                "8F3B4C1A-0D2E-4F5A-9B6C-7D8E9F0A1B2C",
                "web01",
                Some(2),
                Some(5),
            ),
            row(
                "1A2B3C4D-5E6F-4A7B-8C9D-0E1F2A3B4C5D",
                "Backup",
                Some(3),
                Some(20),
            ),
            row("0F0E0D0C-0B0A-4908-8706-050403020100", " ", None, Some(25)),
        ]);
        assert_eq!(
            vms,
            [
                Vm {
                    name: "0F0E0D0C-0B0A-4908-8706-050403020100".into(),
                    power: Power::Other(0),
                    condition: Condition::Critical,
                },
                Vm {
                    name: "Backup".into(),
                    power: Power::Off,
                    condition: Condition::Failure,
                },
                vm("web01", Power::Running),
            ]
        );
        for (state, power) in [
            (6, Power::Saved),
            (32769, Power::Saved),
            (9, Power::Paused),
            (32768, Power::Paused),
            (10, Power::Starting),
            (32777, Power::Starting),
            (4, Power::Stopping),
            (32773, Power::Stopping),
            (1, Power::Other(1)),
        ] {
            assert_eq!(Power::of(state), power, "{state}");
        }
        assert_eq!(Condition::of(None), Condition::Fine);
        assert_eq!(Power::Running.said(), "is running");
    }

    /// By hand (`--ignored --nocapture`), as an administrator: what this
    /// machine's Hyper-V lists. `HYPERV_TEST_VMS` names the machines that
    /// must run, separated by commas.
    #[tokio::test]
    #[ignore = "reads this machine's Hyper-V: run by hand"]
    async fn this_machines_hyperv() {
        let wanted: Vec<String> = std::env::var("HYPERV_TEST_VMS")
            .unwrap_or_default()
            .split(',')
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        match platform::machines().await {
            Ok(vms) => {
                for vm in &vms {
                    println!("{vm:?}");
                }
                let outcome = describe(&vms, &wanted);
                println!("{:?}: {}", outcome.status, outcome.message);
                println!("{:?}", outcome.metrics);
            }
            Err(why) => println!("Down: {why}"),
        }
    }
}
