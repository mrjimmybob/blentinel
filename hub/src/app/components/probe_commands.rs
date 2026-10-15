//! A probe's latest commands and what came of them (M2-06, D-56), on the
//! probe page. A command waits for the probe's next report: the hub never
//! connects to a probe.

use leptos::prelude::*;

use crate::app::ui::fmt_ts;
use crate::model::{CommandInfo, CommandState};

/// What a command kind is called on the page.
pub(crate) fn command_label(kind: &str) -> &str {
    match kind {
        "run_checks" => "Run checks now",
        "discover" => "Discover network",
        other => other,
    }
}

/// The state badge: class and text.
pub(crate) fn state_badge(state: CommandState) -> (&'static str, &'static str) {
    match state {
        CommandState::Queued => (
            "command-badge pending",
            "Waiting for the probe's next report",
        ),
        CommandState::Delivered => ("command-badge pending", "Sent to the probe"),
        CommandState::Succeeded => ("command-badge in-sync", "Done"),
        CommandState::Failed => ("command-badge rejected", "Failed"),
        CommandState::Cancelled => ("command-badge neutral", "Cancelled"),
        CommandState::Expired => (
            "command-badge rejected",
            "Expired: the probe didn't report in time",
        ),
    }
}

/// The table of commands, with Cancel for one not sent yet.
#[component]
pub(crate) fn ProbeCommands(
    commands: RwSignal<Vec<CommandInfo>>,
    can_manage: Signal<bool>,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <Show when=move || !commands.get().is_empty()>
            <div class="admin-section">
                <h2>"Commands"</h2>
                <table class="admin-table commands-table">
                    <thead>
                        <tr>
                            <th>"Command"</th><th>"Queued"</th><th>"State"</th>
                            <th>"Result"</th><th></th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || commands.get().into_iter().map(|c| {
                            let (class, text) = state_badge(c.state);
                            let queued = match &c.created_by {
                                Some(by) => format!("{} by {by}", fmt_ts(&c.created_at)),
                                None => fmt_ts(&c.created_at),
                            };
                            let cancel_url = format!("/api/commands/{}/cancel", c.id);
                            let cancellable = c.state == CommandState::Queued;
                            view! {
                                <tr>
                                    <td>{command_label(&c.kind).to_string()}</td>
                                    <td>{queued}</td>
                                    <td><span class=class>{text}</span></td>
                                    <td>{c.result.clone().unwrap_or_default()}</td>
                                    <td class="row-actions">
                                        <Show when=move || can_manage.get() && cancellable>
                                            {
                                                let url = cancel_url.clone();
                                                view! {
                                                    <button class="btn btn-secondary btn-sm"
                                                        on:click=move |_| act(url.clone(), "{}".into())>
                                                        "Cancel"
                                                    </button>
                                                }
                                            }
                                        </Show>
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
                <p class="info-text">
                    "A command reaches the probe with the answer to its next report, and its result comes with the report after that."
                </p>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_state_says_where_the_command_is() {
        assert_eq!(
            state_badge(CommandState::Queued).1,
            "Waiting for the probe's next report"
        );
        assert_eq!(
            state_badge(CommandState::Succeeded).0,
            "command-badge in-sync"
        );
        assert!(state_badge(CommandState::Expired).1.starts_with("Expired"));
        assert_eq!(command_label("run_checks"), "Run checks now");
        assert_eq!(command_label("discover"), "Discover network");
        assert_eq!(command_label("later_kind"), "later_kind");
    }
}
