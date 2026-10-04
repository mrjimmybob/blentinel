//! Commands from the hub (protocol v6, M2-06, D-56): run what the hub's
//! signed answer asks for, and hand the results to the next report.
//!
//! The hub sends a command in every answer until it gets the result, so the
//! probe remembers the commands it ran and doesn't run them again
//! (`common::commands`: delivery is at least once).

use std::collections::{HashSet, VecDeque};

use common::commands::{Command, CommandResult, CommandSpec, MAX_COMMANDS_PER_MESSAGE};
use tracing::{info, warn};

/// How many command IDs the probe remembers having run: far more than can
/// be pending at once.
const REMEMBERED: usize = 256;

/// What the monitoring loop should do after the commands of one answer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Actions {
    /// Run the checks and report now, not an interval later.
    pub run_now: bool,
}

#[derive(Debug, Default)]
pub struct Commands {
    /// Results for the next report.
    results: Vec<CommandResult>,
    /// IDs of the commands run, oldest first.
    done: VecDeque<String>,
    done_set: HashSet<String>,
}

impl Commands {
    /// Run the commands of an answer that this probe hasn't run yet.
    /// `checks` is how many checks the probe runs now (after any new config
    /// in the same answer).
    pub fn receive(&mut self, commands: &[Command], checks: usize) -> Actions {
        let mut actions = Actions::default();
        for command in commands.iter().take(MAX_COMMANDS_PER_MESSAGE) {
            if !self.remember(&command.id) {
                continue;
            }
            let result = match command.spec {
                CommandSpec::RunChecks => {
                    info!(command = %command.id, "The hub asked to run the checks now");
                    actions.run_now = true;
                    CommandResult::new(&command.id, true, run_checks_message(checks))
                }
                CommandSpec::Unknown => {
                    warn!(command = %command.id, "The hub sent a command this probe doesn't know");
                    CommandResult::new(
                        &command.id,
                        false,
                        format!(
                            "probe {} doesn't know this command: update it",
                            crate::update::VERSION
                        ),
                    )
                }
            };
            self.results.push(result);
        }
        actions
    }

    /// The results for the report being made (then forgotten: the report
    /// carries them, buffered if the hub is unreachable).
    pub fn take_results(&mut self) -> Vec<CommandResult> {
        let take = self.results.len().min(MAX_COMMANDS_PER_MESSAGE);
        self.results.drain(..take).collect()
    }

    /// Note `id` as run; `false` if it already was.
    fn remember(&mut self, id: &str) -> bool {
        if !self.done_set.insert(id.to_string()) {
            return false;
        }
        self.done.push_back(id.to_string());
        if self.done.len() > REMEMBERED
            && let Some(oldest) = self.done.pop_front()
        {
            self.done_set.remove(&oldest);
        }
        true
    }
}

fn run_checks_message(checks: usize) -> String {
    match checks {
        0 => "no checks to run".to_string(),
        1 => "ran 1 check".to_string(),
        n => format!("ran {n} checks"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(id: &str, spec: CommandSpec) -> Command {
        Command {
            id: id.into(),
            spec,
        }
    }

    #[test]
    fn run_checks_runs_now_and_reports_once() {
        let mut commands = Commands::default();
        let actions = commands.receive(&[command("a", CommandSpec::RunChecks)], 3);
        assert!(actions.run_now);
        assert_eq!(
            commands.take_results(),
            [CommandResult::new("a", true, "ran 3 checks")]
        );
        assert!(commands.take_results().is_empty(), "taken once");
    }

    #[test]
    fn a_command_sent_again_is_not_run_again() {
        let mut commands = Commands::default();
        commands.receive(&[command("a", CommandSpec::RunChecks)], 1);
        commands.take_results();
        let again = commands.receive(&[command("a", CommandSpec::RunChecks)], 1);
        assert!(!again.run_now);
        assert!(commands.take_results().is_empty());
    }

    #[test]
    fn an_unknown_command_fails_with_a_reason() {
        let mut commands = Commands::default();
        let actions = commands.receive(&[command("x", CommandSpec::Unknown)], 1);
        assert!(!actions.run_now);
        let results = commands.take_results();
        assert!(!results[0].ok);
        assert!(results[0].message.contains("update it"), "{results:?}");
    }

    #[test]
    fn the_memory_of_run_commands_is_bounded() {
        let mut commands = Commands::default();
        for i in 0..REMEMBERED + 5 {
            commands.receive(&[command(&i.to_string(), CommandSpec::RunChecks)], 0);
            commands.take_results();
        }
        assert_eq!(commands.done.len(), REMEMBERED);
        assert_eq!(commands.done_set.len(), REMEMBERED);
        // The oldest are forgotten; the newest are still known.
        assert!(!commands.done_set.contains("0"));
        let newest = (REMEMBERED + 4).to_string();
        assert!(!commands.remember(&newest));
    }

    #[test]
    fn a_report_carries_a_bounded_number_of_results() {
        let mut commands = Commands::default();
        let many: Vec<_> = (0..MAX_COMMANDS_PER_MESSAGE)
            .map(|i| command(&format!("a{i}"), CommandSpec::RunChecks))
            .collect();
        commands.receive(&many, 1);
        commands.receive(&[command("b", CommandSpec::RunChecks)], 1);
        assert_eq!(commands.take_results().len(), MAX_COMMANDS_PER_MESSAGE);
        assert_eq!(commands.take_results().len(), 1, "the rest go next time");
    }
}
