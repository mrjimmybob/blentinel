//! The probe page's pieces for M5-12 (D-85): what the probe's machine is
//! made of, the checks that suggests, and the templates dialog (add a
//! template's checks to the probe, save the probe's checks as one).

use std::collections::BTreeMap;

use leptos::prelude::*;

use crate::app::components::kind_label;
#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
use crate::app::ui::fmt_ts;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
#[cfg(any(not(feature = "ssr"), test))]
use crate::model::AddedChecks;
use crate::model::{MachineInfo, Suggestion, TemplateInfo};

/// A disk's size for people: `932 GB`, `1.8 TB`.
fn size_text(total_mb: u64) -> String {
    const GB: u64 = 1024;
    const TB: u64 = 1024 * 1024;
    if total_mb >= TB {
        format!("{:.1} TB", total_mb as f64 / TB as f64)
    } else if total_mb >= GB {
        format!("{} GB", (total_mb as f64 / GB as f64).round())
    } else {
        format!("{total_mb} MB")
    }
}

/// What adding several checks did, in words.
#[cfg(any(not(feature = "ssr"), test))]
fn added_text(result: &AddedChecks) -> String {
    let checks = |n: usize| {
        if n == 1 {
            "1 check".to_string()
        } else {
            format!("{n} checks")
        }
    };
    match (result.added.len(), result.skipped) {
        (0, _) => "Nothing added: the probe already has these checks.".to_string(),
        (added, 0) => format!("Added {}.", checks(added)),
        (added, skipped) => format!(
            "Added {}; the probe already had the other {skipped}.",
            checks(added)
        ),
    }
}

/// The body of `POST /api/probes/{id}/checks/batch` for these suggestions.
fn batch_body(suggestions: &[Suggestion]) -> String {
    let checks: Vec<_> = suggestions
        .iter()
        .map(|s| serde_json::json!({ "name": s.name, "spec": s.spec }))
        .collect();
    serde_json::json!({ "checks": checks }).to_string()
}

/// What the probe last said its machine is made of.
#[component]
pub(crate) fn MachineSection(machine: MachineInfo) -> impl IntoView {
    let body = match machine.inventory {
        None => view! {
            <p class="info-text">
                "Not known yet: a probe says what its machine is made of when it starts and when that changes; an older probe does once it updates."
            </p>
        }
        .into_any(),
        Some(inventory) => {
            let system = [
                inventory.os.name.clone(),
                inventory.os.version.clone(),
                inventory.os.arch.clone(),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            let disks = inventory
                .volumes
                .iter()
                .map(|v| format!("{} ({})", v.mount, size_text(v.total_mb)))
                .collect::<Vec<_>>()
                .join(", ");
            let network = inventory
                .interfaces
                .iter()
                .map(|i| format!("{} {}", i.name, i.addresses.join(", ")))
                .collect::<Vec<_>>()
                .join(" · ");
            let or_none = |text: String| if text.is_empty() { "None found".to_string() } else { text };
            let running = inventory.services.len();
            let services = inventory.services.clone();
            view! {
                <dl class="machine-inventory">
                    <dt>"System"</dt><dd>{or_none(system)}</dd>
                    <dt>"Fixed disks"</dt><dd>{or_none(disks)}</dd>
                    <dt>"Network"</dt><dd>{or_none(network)}</dd>
                    <dt>"Default gateway"</dt>
                    <dd>{inventory.gateway.clone().unwrap_or_else(|| "None found".into())}</dd>
                    <dt>"Services"</dt>
                    <dd>
                        <details class="machine-services">
                            <summary>{format!("{running} running")}</summary>
                            <ul>
                                {services.into_iter().map(|s| view! {
                                    <li>
                                        <code>{s.name}</code>
                                        {s.label.map(|label| format!(" · {label}"))}
                                    </li>
                                }).collect_view()}
                            </ul>
                        </details>
                    </dd>
                    <dt>"As of"</dt>
                    <dd>{machine.reported_at.as_deref().map(fmt_ts).unwrap_or_default()}</dd>
                </dl>
            }
            .into_any()
        }
    };
    view! {
        <div class="admin-section machine-section">
            <h2>"Machine"</h2>
            {body}
        </div>
    }
}

/// The checks the hub proposes for this probe: add one, or all.
#[component]
pub(crate) fn SuggestedChecks(
    probe_id: String,
    suggestions: Vec<Suggestion>,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let url = StoredValue::new(format!("/api/probes/{probe_id}/checks/batch"));
    let all = batch_body(&suggestions);
    view! {
        <div class="suggested-checks">
            <h3>"Suggested checks"</h3>
            <p class="info-text">
                "From what this machine is made of. Nothing is checked until you add it."
            </p>
            <ul>
                {suggestions.into_iter().map(|s| {
                    let body = batch_body(std::slice::from_ref(&s));
                    view! {
                        <li>
                            <button class="btn btn-secondary btn-sm"
                                on:click=move |_| act(url.get_value(), body.clone())>"Add"</button>
                            <strong>{s.name.clone()}</strong>
                            <span class="check-kind">{kind_label(s.spec.kind())}</span>
                            <span class="suggestion-why">{s.why.clone()}</span>
                        </li>
                    }
                }).collect_view()}
            </ul>
            <button class="btn btn-primary btn-sm"
                on:click=move |_| act(url.get_value(), all.clone())>"Add all suggested"</button>
        </div>
    }
}

/// Templates: add one's checks to this probe, save this probe's checks as
/// one, delete a saved one.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn TemplatesDialog(
    probe_id: String,
    /// Has the probe checks to save as a template?
    has_checks: bool,
    open: RwSignal<bool>,
    /// The probe's checks changed: reload them.
    changed: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let probe_id = StoredValue::new(probe_id);
    let templates = RwSignal::new(Vec::<TemplateInfo>::new());
    let error = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    // What was typed for each parameter, by `<template id>:<key>`.
    let values = RwSignal::new(BTreeMap::<String, String>::new());
    let new_name = RwSignal::new(String::new());
    let new_description = RwSignal::new(String::new());

    let load = move || {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match fetch_json::<Vec<TemplateInfo>>("/api/check-templates").await {
                Ok(list) => templates.set(list),
                Err(e) => error.set(e),
            }
        });
    };
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        load();
    });

    let apply = move |template: TemplateInfo| {
        error.set(String::new());
        message.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let typed = values.get_untracked();
                let given: BTreeMap<String, String> = template
                    .parameters
                    .iter()
                    .map(|p| {
                        let value = typed.get(&format!("{}:{}", template.id, p.key));
                        (p.key.clone(), value.cloned().unwrap_or_default())
                    })
                    .collect();
                let body = serde_json::json!({ "template_id": template.id, "values": given });
                let url = format!("/api/probes/{}/checks/template", probe_id.get_value());
                match post_for::<AddedChecks>(&url, &body.to_string()).await {
                    Ok(result) => {
                        message.set(format!("{}: {}", template.name, added_text(&result)));
                        changed();
                    }
                    Err(e) => error.set(e),
                }
            }
        });
    };
    // A POST about the templates themselves, then the list again.
    let manage = move |url: String, body: String, done: &'static str| {
        error.set(String::new());
        message.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json(&url, &body).await {
                Ok(()) => {
                    message.set(done.to_string());
                    new_name.set(String::new());
                    new_description.set(String::new());
                    load();
                }
                Err(e) => error.set(e),
            }
        });
    };
    let save = move || {
        let body = serde_json::json!({
            "name": new_name.get_untracked(),
            "description": new_description.get_untracked(),
            "from_probe_id": probe_id.get_value(),
        });
        manage(
            "/api/check-templates".into(),
            body.to_string(),
            "Template saved.",
        );
    };

    #[cfg(not(feature = "ssr"))]
    let (on_name, on_description) = (
        move |ev| new_name.set(input_value(&ev)),
        move |ev| new_description.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_name, on_description) = (|_| {}, |_| {});

    view! {
        <div class="modal-overlay" on:click=move |_| open.set(false)>
            <div class="modal templates-dialog" on:click=|e| e.stop_propagation()>
                <h3>"Check templates"</h3>
                <p>
                    "A template is a set of checks added to a probe in one go. The checks are then this probe's own: change or delete them as any other."
                </p>
                <div class="login-error">{error}</div>
                <div class="templates-message">{message}</div>
                {move || templates.get().into_iter().map(|template| {
                    let checks = template
                        .checks
                        .iter()
                        .map(|c| c.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let delete_url = format!("/api/check-templates/{}/delete", template.id);
                    let saved = !template.builtin;
                    let by = template.created_by.clone();
                    let to_apply = template.clone();
                    view! {
                        <div class="template-card">
                            <h4>
                                {template.name.clone()}
                                {saved.then(|| view! {
                                    <span class="template-saved">
                                        {format!(" · saved{}", by.map(|b| format!(" by {b}")).unwrap_or_default())}
                                    </span>
                                })}
                            </h4>
                            {(!template.description.is_empty()).then(|| view! {
                                <p class="template-description">{template.description.clone()}</p>
                            })}
                            <p class="template-checks">{format!("Checks: {checks}")}</p>
                            {template.parameters.clone().into_iter().map(|parameter| {
                                let key = format!("{}:{}", template.id, parameter.key);
                                let (shown, typed) = (key.clone(), key.clone());
                                #[cfg(not(feature = "ssr"))]
                                let on_input = move |ev| {
                                    let value = input_value(&ev);
                                    values.update(|v| { v.insert(typed.clone(), value); });
                                };
                                #[cfg(feature = "ssr")]
                                let on_input = |_| {};
                                view! {
                                    <div class="form-group">
                                        <label for=key.clone()>{parameter.label.clone()}</label>
                                        <input id=key.clone() name=key.clone()
                                            type=if parameter.secret { "password" } else { "text" }
                                            autocomplete="off"
                                            placeholder=parameter.example.clone()
                                            prop:value=move || values.with(|v| v.get(&shown).cloned().unwrap_or_default())
                                            on:input=on_input/>
                                    </div>
                                }
                            }).collect_view()}
                            <div class="modal-actions">
                                {saved.then(|| view! {
                                    <button class="btn btn-danger btn-sm" on:click=move |_| {
                                        manage(delete_url.clone(), "{}".into(), "Template deleted.")
                                    }>"Delete"</button>
                                })}
                                <button class="btn btn-primary btn-sm"
                                    on:click=move |_| apply(to_apply.clone())>"Add to this probe"</button>
                            </div>
                        </div>
                    }
                }).collect_view()}
                {has_checks.then(|| view! {
                    <div class="template-card template-save">
                        <h4>"Save this probe's checks as a template"</h4>
                        <p class="template-description">
                            "Each check's name and what it checks, as they are now. Saved templates are for users who see every company."
                        </p>
                        <div class="form-group">
                            <label for="template_name">"Name"</label>
                            <input id="template_name" name="template_name" type="text"
                                placeholder="Branch office"
                                prop:value=move || new_name.get() on:input=on_name/>
                        </div>
                        <div class="form-group">
                            <label for="template_description">"Description (optional)"</label>
                            <input id="template_description" name="template_description" type="text"
                                prop:value=move || new_description.get() on:input=on_description/>
                        </div>
                        <div class="modal-actions">
                            <button class="btn btn-secondary btn-sm" on:click=move |_| save()>
                                "Save as template"
                            </button>
                        </div>
                    </div>
                })}
                <div class="modal-actions">
                    <button class="btn btn-secondary" on:click=move |_| open.set(false)>"Close"</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::CheckSpec;

    #[test]
    fn a_disks_size_is_said_in_the_unit_that_fits() {
        assert_eq!(size_text(512), "512 MB");
        assert_eq!(size_text(50_000), "49 GB");
        assert_eq!(size_text(953_674), "931 GB");
        assert_eq!(size_text(1_907_348), "1.8 TB");
    }

    fn added(added: usize, skipped: usize) -> AddedChecks {
        let check: crate::model::CheckInfo = serde_json::from_value(serde_json::json!({
            "id": "c", "probe_id": "p", "name": "CPU", "spec": { "kind": "cpu" },
            "target": "localhost", "enabled": true, "fail_after": null, "recover_after": null,
            "depends_on": null, "created_at": "", "updated_at": "", "updated_by": null,
        }))
        .unwrap();
        AddedChecks {
            added: vec![check; added],
            skipped,
        }
    }

    #[test]
    fn what_was_added_is_said_in_words() {
        assert_eq!(added_text(&added(3, 0)), "Added 3 checks.");
        assert_eq!(
            added_text(&added(1, 2)),
            "Added 1 check; the probe already had the other 2."
        );
        assert_eq!(
            added_text(&added(0, 4)),
            "Nothing added: the probe already has these checks."
        );
    }

    #[test]
    fn suggestions_are_sent_as_checks_with_a_name_and_a_spec() {
        let suggestions = [Suggestion {
            name: "Gateway".into(),
            spec: CheckSpec::ping("192.168.1.1"),
            why: "the machine's default gateway".into(),
        }];
        let body: crate::model::AddChecksRequest =
            serde_json::from_str(&batch_body(&suggestions)).unwrap();
        assert_eq!(body.checks.len(), 1);
        assert_eq!(body.checks[0].name, "Gateway");
        assert_eq!(body.checks[0].spec, CheckSpec::ping("192.168.1.1"));
        assert!(body.checks[0].enabled);
    }
}
