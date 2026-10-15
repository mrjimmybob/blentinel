//! The probe page's "Network" section (M5-18, D-101): ask the probe to look
//! at its network once, and what it found there last time: the devices
//! that answered, and the checks they suggest.

use leptos::prelude::*;

use super::machine::batch_body;
use crate::app::ui::fmt_ts;
use crate::model::{FoundDevice, NetworkInfo};

/// The ports of a device, for people: `445 Windows file sharing, 5000 NAS
/// web interface`.
fn ports_text(ports: &[u16]) -> String {
    ports
        .iter()
        .map(|port| match common::discovery::port_label(*port) {
            Some(label) => format!("{port} {label}"),
            None => port.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a sweep came to, in a line: `12 devices on 192.168.1.0/24 · 253
/// addresses tried in 9 s`.
fn summary(network: &NetworkInfo) -> String {
    let devices = match network.devices.len() {
        0 => "No device answered".to_string(),
        1 => "1 device".to_string(),
        n => format!("{n} devices"),
    };
    let place = if network.networks.is_empty() {
        String::new()
    } else {
        format!(" on {}", network.networks.join(", "))
    };
    let tried = match network.scanned {
        0 => "no address tried".to_string(),
        1 => "1 address tried".to_string(),
        n => format!("{n} addresses tried"),
    };
    format!("{devices}{place} · {tried} in {} s", network.seconds)
}

/// How many of the probe's checks watch a device, in words.
fn watched_text(checks: usize) -> String {
    match checks {
        0 => "None".to_string(),
        1 => "1 check".to_string(),
        n => format!("{n} checks"),
    }
}

/// The button that adds what a device suggests.
fn add_text(suggestions: usize) -> String {
    match suggestions {
        1 => "Add 1 check".to_string(),
        n => format!("Add {n} checks"),
    }
}

/// What the probe found on its network, and the button to look again.
#[component]
pub(crate) fn NetworkSection(
    probe_id: String,
    network: NetworkInfo,
    /// May this user send the probe commands and add checks?
    can_manage: Signal<bool>,
    /// Is the probe active (a revoked one takes no commands)?
    active: bool,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let commands_url = StoredValue::new(format!("/api/probes/{probe_id}/commands"));
    let batch_url = StoredValue::new(format!("/api/probes/{probe_id}/checks/batch"));
    let allowed = network.allowed;
    let found = network.reported_at.is_some();
    let line = summary(&network);
    let as_of = network
        .reported_at
        .as_deref()
        .map(fmt_ts)
        .unwrap_or_default();
    let note = network.note.clone();
    let devices = network.devices.clone();

    let row = move |found: FoundDevice| {
        let FoundDevice {
            device,
            checks,
            suggestions,
        } = found;
        let body = batch_body(&suggestions);
        let offered = suggestions.len();
        let any = offered > 0;
        let title = suggestions
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        view! {
            <tr>
                <td><code>{device.address.clone()}</code></td>
                <td>{device.name.clone().unwrap_or_default()}</td>
                <td><code>{device.mac.clone().unwrap_or_default()}</code></td>
                <td>{if device.pinged { "Yes" } else { "No" }}</td>
                <td>{ports_text(&device.ports)}</td>
                <td>{watched_text(checks)}</td>
                <td>
                    <Show when=move || can_manage.get() && any>
                        <button class="btn btn-secondary btn-sm" title=title.clone()
                            on:click={
                                let body = body.clone();
                                move |_| act(batch_url.get_value(), body.clone())
                            }>
                            {add_text(offered)}
                        </button>
                    </Show>
                </td>
            </tr>
        }
    };

    view! {
        <div class="admin-section network-section">
            <h2>"Network"</h2>
            {if allowed {
                view! {
                    <p class="info-text">
                        "The probe can look at its own network once: which addresses answer, and on which well-known ports. Private networks only, at most 1024 addresses. Nothing is checked until you add it."
                    </p>
                    <Show when=move || can_manage.get()>
                        <div class="section-actions">
                            <button class="btn btn-secondary" id="discover_network" disabled=!active
                                title="The probe gets this with the answer to its next report, looks for a minute or so, and reports what it found"
                                on:click=move |_| act(commands_url.get_value(), r#"{"kind":"discover"}"#.into())>
                                "Discover network"
                            </button>
                        </div>
                    </Show>
                }.into_any()
            } else {
                view! {
                    <p class="info-text" id="discovery_off">
                        "Network discovery is off for this company: some networks' owners forbid scanning. An Admin allows it on the company's page."
                    </p>
                }.into_any()
            }}
            {found.then(|| view! {
                <p class="network-summary">
                    <strong>{line}</strong>
                    <span class="network-as-of">" · as of "{as_of}</span>
                </p>
                {note.map(|note| view! { <p class="info-text network-note">{note}</p> })}
                {(!devices.is_empty()).then(|| view! {
                    <table class="admin-table network-table">
                        <thead>
                            <tr>
                                <th>"Address"</th><th>"Name"</th><th>"Hardware address"</th>
                                <th>"Answers ping"</th><th>"Listens on"</th><th>"Checked by"</th><th></th>
                            </tr>
                        </thead>
                        <tbody>{devices.into_iter().map(row).collect_view()}</tbody>
                    </table>
                })}
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_is_summed_up_in_a_line() {
        let mut network = NetworkInfo {
            allowed: true,
            reported_at: Some("2026-10-14T09:00:00Z".into()),
            networks: vec!["192.168.1.0/24".into()],
            scanned: 253,
            seconds: 9,
            ..NetworkInfo::default()
        };
        assert_eq!(
            summary(&network),
            "No device answered on 192.168.1.0/24 · 253 addresses tried in 9 s"
        );
        network.networks.clear();
        network.scanned = 0;
        assert_eq!(
            summary(&network),
            "No device answered · no address tried in 9 s"
        );
    }

    #[test]
    fn ports_and_counts_read_well() {
        assert_eq!(
            ports_text(&[445, 5000, 31337]),
            "445 Windows file sharing, 5000 NAS web interface, 31337"
        );
        assert_eq!(ports_text(&[]), "");
        assert_eq!(watched_text(0), "None");
        assert_eq!(watched_text(1), "1 check");
        assert_eq!(watched_text(3), "3 checks");
        assert_eq!(add_text(1), "Add 1 check");
        assert_eq!(add_text(4), "Add 4 checks");
    }
}
