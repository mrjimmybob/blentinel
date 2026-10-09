//! The command channel (protocol v6, M2-06, proposal 0003 §4.3, D-56).
//!
//! The hub queues a command for a probe; it rides in the signed answer to
//! the probe's next report ([`crate::ack::ReportAck::commands`]), and the
//! probe sends its result in the report after that
//! ([`crate::models::StatusReport::command_results`]). The hub never
//! connects to the probe (FP-1); the probe also asks for its commands
//! between reports (`common::poll`, D-76), so they usually arrive within
//! seconds, and at worst with the next report.
//!
//! Delivery is **at least once**: the hub sends a command in every answer
//! until its result arrives or it expires, and the probe skips a command it
//! has already run. A probe that restarts in between may run it twice, so
//! every command must be harmless to repeat.

use serde::{Deserialize, Serialize};

/// The most commands one answer carries, and results one report carries.
pub const MAX_COMMANDS_PER_MESSAGE: usize = 10;

/// The longest result message kept (characters).
pub const MAX_RESULT_MESSAGE_CHARS: usize = 500;

/// What a command asks the probe to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandSpec {
    /// Run every check now and report at once, instead of an interval later.
    RunChecks,
    /// A kind this probe doesn't know (sent by a newer hub). The probe
    /// answers that it can't run it, rather than refusing the whole answer.
    #[serde(other)]
    Unknown,
}

impl CommandSpec {
    /// The `kind` as stored and shown.
    pub fn kind(&self) -> &'static str {
        match self {
            CommandSpec::RunChecks => "run_checks",
            CommandSpec::Unknown => "unknown",
        }
    }
}

/// One command, as the hub sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Chosen by the hub; the result names it.
    pub id: String,
    #[serde(flatten)]
    pub spec: CommandSpec,
}

/// What came of a command, as the probe reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    pub id: String,
    pub ok: bool,
    /// For people (at most [`MAX_RESULT_MESSAGE_CHARS`]).
    pub message: String,
}

impl CommandResult {
    pub fn new(id: &str, ok: bool, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            ok,
            message: truncate(&message.into()),
        }
    }
}

/// `message` cut to [`MAX_RESULT_MESSAGE_CHARS`] characters.
pub fn truncate(message: &str) -> String {
    message.chars().take(MAX_RESULT_MESSAGE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_format_is_flat() {
        let command = Command {
            id: "c1".into(),
            spec: CommandSpec::RunChecks,
        };
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(json, r#"{"id":"c1","kind":"run_checks"}"#);
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
    }

    #[test]
    fn a_kind_from_a_newer_hub_is_unknown_not_an_error() {
        let command: Command =
            serde_json::from_str(r#"{"id":"c2","kind":"self_destruct","delay":5}"#).unwrap();
        assert_eq!(command.spec, CommandSpec::Unknown);
        assert_eq!(command.id, "c2");
    }

    #[test]
    fn result_messages_are_capped() {
        let result = CommandResult::new("c1", true, "é".repeat(MAX_RESULT_MESSAGE_CHARS + 10));
        assert_eq!(result.message.chars().count(), MAX_RESULT_MESSAGE_CHARS);
    }
}
