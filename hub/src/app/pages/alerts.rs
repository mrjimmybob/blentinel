//! The Alerts page (M3-03, D-66): the hub's mail server and address, the
//! channels alerts go to (with "Send test"), and the routing rules that say
//! which alerts go where. Owners and Admins; secrets are write-only.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::model::TestResult;
use crate::model::{
    AlertKind, AlertSettings, AlertsOverview, ChannelInfo, ChannelKind, ChannelRequest,
    MailSecurity, Permission, RouteRule, RoutesRequest,
};

#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
pub(crate) fn AlertsPage() -> impl IntoView {
    let me = Me::current();
    let can_edit = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());

    let overview: LocalResource<Result<AlertsOverview, String>> = LocalResource::new(move || {
        refresh.track();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<AlertsOverview>("/api/admin/alerts").await
            }
            #[cfg(feature = "ssr")]
            {
                Err(String::new())
            }
        }
    });

    // POST `body` to `url`, then reload (or show the hub's reason).
    let act = move |url: String, body: String, done: &'static str| {
        error.set(String::new());
        message.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json(&url, &body).await {
                Ok(()) => {
                    message.set(done.into());
                    refresh.update(|n| *n += 1);
                }
                Err(e) => error.set(e),
            }
            #[cfg(feature = "ssr")]
            let _ = (url, body, done);
        });
    };

    view! {
        <h1 class="page-title">"Alerts"</h1>
        <p class="info-text">
            "Where alerts go. Channels are e-mail, a signed webhook, Teams, Slack, Discord or Telegram; \
             the routing rules decide which alerts each one gets. Thresholds are on the admin and company pages."
        </p>
        <div class="login-error">{error}</div>
        <div class="form-message">{message}</div>
        {move || -> AnyView {
            match overview.get() {
                Some(Ok(data)) => any(view! {
                    <Channels data=data.clone() can_edit error message act refresh/>
                    <Rules data=data.clone() can_edit error act/>
                    <Settings settings=data.settings can_edit error act/>
                }),
                Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                None => any(view! { <div class="loading">"Loading…"</div> }),
            }
        }}
    }
}

/// The channels, and a form to add or change one.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Channels(
    data: AlertsOverview,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    message: RwSignal<String>,
    act: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
    refresh: RwSignal<u32>,
) -> impl IntoView {
    // The form: `editing` is the channel changed (`None`: a new one).
    let editing = RwSignal::new(None::<i64>);
    let name = RwSignal::new(String::new());
    let kind = RwSignal::new(ChannelKind::Email);
    let target = RwSignal::new(String::new());
    let secret = RwSignal::new(String::new());
    let enabled = RwSignal::new(true);
    let secret_set = RwSignal::new(false);

    let edit = move |c: ChannelInfo| {
        editing.set(Some(c.id));
        name.set(c.name);
        kind.set(c.kind);
        target.set(c.target);
        secret.set(String::new());
        enabled.set(c.enabled);
        secret_set.set(c.secret_set);
    };
    let reset = move || {
        editing.set(None);
        name.set(String::new());
        target.set(String::new());
        secret.set(String::new());
        enabled.set(true);
        secret_set.set(false);
    };
    let test = move |id: i64| {
        error.set(String::new());
        message.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_for::<TestResult>(&format!("/api/admin/alerts/channels/{id}/test"), "{}")
                .await
            {
                Ok(TestResult { ok: true, .. }) => message.set("Test sent.".into()),
                Ok(TestResult { error: e, .. }) => {
                    error.set(format!("Test failed: {}", e.unwrap_or_default()))
                }
                Err(e) => error.set(e),
            }
            #[cfg(feature = "ssr")]
            let _ = id;
            refresh.update(|n| *n += 1);
        });
    };
    let save = move |_| {
        let request = ChannelRequest {
            name: name.get_untracked(),
            kind: kind.get_untracked(),
            enabled: enabled.get_untracked(),
            target: target.get_untracked(),
            secret: Some(secret.get_untracked()).filter(|s| !s.trim().is_empty()),
        };
        let has_secret = editing.get_untracked().is_some() && secret_set.get_untracked();
        if let Err(e) = request.validate(has_secret) {
            error.set(format!("Not saved: {e}."));
            return;
        }
        let url = match editing.get_untracked() {
            Some(id) => format!("/api/admin/alerts/channels/{id}"),
            None => "/api/admin/alerts/channels".to_string(),
        };
        act(
            url,
            serde_json::to_string(&request).unwrap_or_default(),
            "Channel saved.",
        );
        reset();
    };

    let rows = data
        .channels
        .iter()
        .map(|c| {
            let id = c.id;
            let for_edit = c.clone();
            let last = match (&c.last_delivery_at, &c.last_error) {
                (None, _) => "never".to_string(),
                (Some(at), None) => format!("{at}: delivered"),
                (Some(at), Some(e)) => format!("{at}: failed: {e}"),
            };
            view! {
                <tr class:inactive=!c.enabled>
                    <td>{c.name.clone()}</td>
                    <td>{c.kind.label()}</td>
                    <td>{c.target.clone()}{c.secret_set.then_some(" (secret set)")}</td>
                    <td class:error-text=c.last_error.is_some()>{last}</td>
                    <td class="row-actions">
                        <Show when=move || can_edit.get()>
                            <button class="btn btn-secondary btn-sm" name="channel_test" on:click=move |_| test(id)>"Send test"</button>
                            <button class="btn btn-secondary btn-sm" name="channel_edit"
                                on:click={ let c = for_edit.clone(); move |_| edit(c.clone()) }>"Edit"</button>
                            <button class="btn btn-secondary btn-sm" name="channel_delete"
                                on:click=move |_| act(format!("/api/admin/alerts/channels/{id}/delete"), "{}".into(), "Channel removed.")>
                                "Remove"
                            </button>
                        </Show>
                    </td>
                </tr>
            }
        })
        .collect_view();

    #[cfg(not(feature = "ssr"))]
    let set = |signal: RwSignal<String>| move |ev| signal.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let set = |_signal: RwSignal<String>| |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_kind =
        move |ev| kind.set(ChannelKind::parse(&input_value(&ev)).unwrap_or(ChannelKind::Email));
    #[cfg(feature = "ssr")]
    let on_kind = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_enabled = move |ev| enabled.set(event_target_checked(&ev));
    #[cfg(feature = "ssr")]
    let on_enabled = |_| {};

    view! {
        <div class="admin-section alerts-channels">
            <h2>"Channels"</h2>
            {match data.channels.is_empty() {
                true => any(view! { <div class="empty-state">"No channels yet: nothing is sent. Add one below, then a rule."</div> }),
                false => any(view! {
                    <table class="admin-table channels-table">
                        <thead><tr><th>"Name"</th><th>"Kind"</th><th>"To"</th><th>"Last delivery"</th><th></th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                }),
            }}
            <Show when=move || can_edit.get()>
                <div class="channel-form">
                    <h3>{move || if editing.get().is_some() { "Change channel" } else { "Add a channel" }}</h3>
                    <label>"Name "<input type="text" name="channel_name" prop:value=move || name.get() on:input=set(name)/></label>
                    <label>"Kind "
                        <select name="channel_kind" prop:value=move || kind.get().as_str() on:change=on_kind>
                            {ChannelKind::ALL.into_iter().map(|k| view! { <option value=k.as_str()>{k.label()}</option> }).collect_view()}
                        </select>
                    </label>
                    {move || kind.get().target_label().map(|label| view! {
                        <label>{label}" "<input type="text" name="channel_target" prop:value=move || target.get() on:input=set(target)/></label>
                    })}
                    {move || kind.get().secret_label().map(|label| view! {
                        <label>{label}" "
                            <input type="password" name="channel_secret" autocomplete="off"
                                placeholder=move || if secret_set.get() { "(set: leave empty to keep it)" } else { "" }
                                prop:value=move || secret.get() on:input=set(secret)/>
                        </label>
                    })}
                    <label><input type="checkbox" name="channel_enabled" prop:checked=move || enabled.get() on:change=on_enabled/>" Enabled"</label>
                    <div class="form-actions">
                        <button class="btn btn-primary btn-sm" name="channel_save" on:click=save>"Save channel"</button>
                        <Show when=move || editing.get().is_some()>
                            <button class="btn btn-secondary btn-sm" on:click=move |_| reset()>"Cancel"</button>
                        </Show>
                    </div>
                </div>
            </Show>
        </div>
    }
}

/// One choice of a rule: a checkbox that adds or removes `value` in a list.
fn toggle<T: PartialEq + Clone + Send + Sync + 'static>(
    rules: RwSignal<Vec<RouteRule>>,
    index: usize,
    list: fn(&mut RouteRule) -> &mut Vec<T>,
    value: T,
    label: String,
    checked: bool,
) -> impl IntoView {
    #[cfg(not(feature = "ssr"))]
    let on_change = move |ev| {
        let on = event_target_checked(&ev);
        let value = value.clone();
        rules.update(|r| {
            let items = list(&mut r[index]);
            items.retain(|v| *v != value);
            if on {
                items.push(value);
            }
        });
    };
    #[cfg(feature = "ssr")]
    let on_change = {
        let _ = (rules, index, list, value);
        |_| {}
    };
    view! { <label class="rule-choice"><input type="checkbox" prop:checked=checked on:change=on_change/>{label}</label> }
}

/// The routing rules, edited as a list, then saved together.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Rules(
    data: AlertsOverview,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    act: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let rules = RwSignal::new(data.rules.clone());
    let data = StoredValue::new(data);
    let save = move |_| {
        let request = RoutesRequest {
            rules: rules.get_untracked(),
        };
        let ids: Vec<i64> = data.with_value(|d| d.channels.iter().map(|c| c.id).collect());
        if let Err(e) = request.validate(&ids) {
            error.set(format!("Not saved: {e}."));
            return;
        }
        act(
            "/api/admin/alerts/routes".into(),
            serde_json::to_string(&request).unwrap_or_default(),
            "Rules saved.",
        );
    };
    let add = move |_| rules.update(|r| r.push(RouteRule::default()));

    let rule_views = move || {
        let d = data.get_value();
        rules
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, rule)| {
                #[cfg(not(feature = "ssr"))]
                let on_name = move |ev| {
                    let value = input_value(&ev);
                    rules.update(|r| r[i].name = value);
                };
                #[cfg(feature = "ssr")]
                let on_name = |_| {};
                #[cfg(not(feature = "ssr"))]
                let on_stop = move |ev| {
                    let on = event_target_checked(&ev);
                    rules.update(|r| r[i].stop = on);
                };
                #[cfg(feature = "ssr")]
                let on_stop = |_| {};
                let companies = d
                    .companies
                    .iter()
                    .map(|c| {
                        toggle(rules, i, |r| &mut r.companies, c.company_id.clone(), c.company_name.clone(), rule.companies.contains(&c.company_id))
                    })
                    .collect_view();
                let sites = d
                    .companies
                    .iter()
                    .flat_map(|c| {
                        c.sites.iter().map(|(id, name)| {
                            let key = format!("{}/{id}", c.company_id);
                            let checked = rule.sites.contains(&key);
                            toggle(rules, i, |r| &mut r.sites, key, format!("{} › {name}", c.company_name), checked)
                        })
                    })
                    .collect_view();
                let kinds = AlertKind::ALL
                    .into_iter()
                    .map(|k| toggle(rules, i, |r| &mut r.kinds, k, k.label().to_string(), rule.kinds.contains(&k)))
                    .collect_view();
                let check_kinds = common::checks::CHECK_KINDS
                    .iter()
                    .map(|k| {
                        let k = k.to_string();
                        let checked = rule.check_kinds.contains(&k);
                        toggle(rules, i, |r| &mut r.check_kinds, k.clone(), k, checked)
                    })
                    .collect_view();
                let channels = d
                    .channels
                    .iter()
                    .map(|c| toggle(rules, i, |r| &mut r.channels, c.id, c.name.clone(), rule.channels.contains(&c.id)))
                    .collect_view();
                // Reminders and escalation (D-69): minutes, empty for never
                // (something not a number fails the hub's range check).
                let minutes = |m: Option<u32>| m.map(|m| m.to_string()).unwrap_or_default();
                #[cfg(not(feature = "ssr"))]
                let typed = |ev: &leptos::ev::Event| {
                    let text = input_value(ev);
                    let text = text.trim();
                    (!text.is_empty()).then(|| text.parse().unwrap_or(0))
                };
                #[cfg(not(feature = "ssr"))]
                let on_remind = move |ev| {
                    let value = typed(&ev);
                    rules.update(|r| r[i].remind_every_minutes = value);
                };
                #[cfg(feature = "ssr")]
                let on_remind = |_| {};
                #[cfg(not(feature = "ssr"))]
                let on_escalate = move |ev| {
                    let value = typed(&ev);
                    rules.update(|r| r[i].escalate_after_minutes = value);
                };
                #[cfg(feature = "ssr")]
                let on_escalate = |_| {};
                let escalate_to = d
                    .channels
                    .iter()
                    .map(|c| toggle(rules, i, |r| &mut r.escalate_to, c.id, c.name.clone(), rule.escalate_to.contains(&c.id)))
                    .collect_view();
                let last = rules.with(|r| r.len()) - 1;
                view! {
                    <div class="rule">
                        <div class="rule-head">
                            <strong>{format!("{}.", i + 1)}</strong>
                            <input type="text" name="rule_name" placeholder="Name (optional)" prop:value=rule.name.clone() on:change=on_name/>
                            <label><input type="checkbox" name="rule_stop" prop:checked=rule.stop on:change=on_stop/>" Stop here when it matches"</label>
                            <Show when=move || can_edit.get()>
                                <button class="btn btn-secondary btn-sm" disabled=i == 0
                                    on:click=move |_| rules.update(|r| r.swap(i, i - 1))>"↑"</button>
                                <button class="btn btn-secondary btn-sm" disabled=i == last
                                    on:click=move |_| rules.update(|r| r.swap(i, i + 1))>"↓"</button>
                                <button class="btn btn-secondary btn-sm" name="rule_delete"
                                    on:click=move |_| rules.update(|r| { r.remove(i); })>"Remove"</button>
                            </Show>
                        </div>
                        <div class="rule-line"><span class="rule-label">"Alerts (none ticked: all)"</span>{kinds}</div>
                        <div class="rule-line"><span class="rule-label">"Companies (none: all)"</span>{companies}</div>
                        <div class="rule-line"><span class="rule-label">"Sites (none: all)"</span>{sites}</div>
                        <div class="rule-line"><span class="rule-label">"Check kinds (none: all)"</span>{check_kinds}</div>
                        <div class="rule-line rule-channels"><span class="rule-label">"Send to"</span>{channels}</div>
                        <div class="rule-line rule-follow-up">
                            <span class="rule-label">"While nobody acknowledges"</span>
                            <label>"remind every "
                                <input type="number" name="rule_remind" min="15" max="10080" size="5"
                                    placeholder="never" prop:value=minutes(rule.remind_every_minutes) on:change=on_remind/>
                                " minutes; "</label>
                            <label>"escalate after "
                                <input type="number" name="rule_escalate" min="5" max="1440" size="5"
                                    placeholder="never" prop:value=minutes(rule.escalate_after_minutes) on:change=on_escalate/>
                                " minutes to:"</label>
                        </div>
                        <div class="rule-line rule-escalate-to"><span class="rule-label">"Escalate to"</span>{escalate_to}</div>
                    </div>
                }
            })
            .collect_view()
    };

    view! {
        <div class="admin-section alerts-rules">
            <h2>"Routing rules"</h2>
            <p class="info-text">
                "Each alert is checked against the rules in order, and goes to the channels of every rule it matches, \
                 unless a matching rule says stop. No rule: nothing is sent."
            </p>
            {move || if rules.with(|r| r.is_empty()) {
                any(view! { <div class="empty-state">"No rules yet."</div> })
            } else {
                nothing()
            }}
            {rule_views}
            <Show when=move || can_edit.get()>
                <div class="form-actions">
                    <button class="btn btn-secondary btn-sm" name="rule_add" on:click=add>"Add rule"</button>
                    <button class="btn btn-primary btn-sm" name="rules_save" on:click=save>"Save rules"</button>
                </div>
            </Show>
        </div>
    }
}

/// The hub's mail server and its address for links.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Settings(
    settings: AlertSettings,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    act: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let hub_url = RwSignal::new(settings.hub_url.clone());
    let server = RwSignal::new(settings.smtp_server.clone());
    let port = RwSignal::new(settings.smtp_port.to_string());
    let security = RwSignal::new(settings.smtp_security.as_str().to_string());
    let username = RwSignal::new(settings.smtp_username.clone());
    let from = RwSignal::new(settings.smtp_from.clone());
    let password = RwSignal::new(String::new());
    let password_set = settings.smtp_password_set;
    #[cfg(not(feature = "ssr"))]
    let set = |signal: RwSignal<String>| move |ev| signal.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let set = |_signal: RwSignal<String>| |_| {};
    let save = move |_| {
        let request = AlertSettings {
            hub_url: hub_url.get_untracked(),
            smtp_server: server.get_untracked(),
            smtp_port: port.get_untracked().trim().parse().unwrap_or(0),
            smtp_security: MailSecurity::parse(&security.get_untracked()).unwrap_or_default(),
            smtp_username: username.get_untracked(),
            smtp_from: from.get_untracked(),
            smtp_password: Some(password.get_untracked()).filter(|p| !p.is_empty()),
            smtp_password_set: false,
        };
        if let Err(e) = request.validate() {
            error.set(format!("Not saved: {e}."));
            return;
        }
        act(
            "/api/admin/alerts/settings".into(),
            serde_json::to_string(&request).unwrap_or_default(),
            "Settings saved.",
        );
    };
    let disabled = move || !can_edit.get();
    view! {
        <div class="admin-section alerts-settings">
            <h2>"Settings"</h2>
            <div class="settings-form">
                <label>"Hub address, for links in alerts "
                    <input type="url" name="hub_url" placeholder="https://hub.example.com" disabled=disabled
                        prop:value=move || hub_url.get() on:input=set(hub_url)/></label>
                <h3>"Mail server (for e-mail channels)"</h3>
                <label>"Server "<input type="text" name="smtp_server" disabled=disabled prop:value=move || server.get() on:input=set(server)/></label>
                <label>"Port "<input type="number" name="smtp_port" disabled=disabled prop:value=move || port.get() on:input=set(port)/></label>
                <label>"Security "
                    <select name="smtp_security" disabled=disabled prop:value=move || security.get() on:change=set(security)>
                        <option value="starttls">"STARTTLS (587)"</option>
                        <option value="tls">"TLS (465)"</option>
                        <option value="none">"None (25, local relay)"</option>
                    </select>
                </label>
                <label>"Username "<input type="text" name="smtp_username" autocomplete="off" disabled=disabled prop:value=move || username.get() on:input=set(username)/></label>
                <label>"Password "
                    <input type="password" name="smtp_password" autocomplete="new-password" disabled=disabled
                        placeholder=if password_set { "(set: leave empty to keep it)" } else { "" }
                        prop:value=move || password.get() on:input=set(password)/></label>
                <label>"From "<input type="email" name="smtp_from" placeholder="hub@example.com" disabled=disabled prop:value=move || from.get() on:input=set(from)/></label>
                <Show when=move || can_edit.get()>
                    <button class="btn btn-primary btn-sm" name="settings_save" on:click=save>"Save settings"</button>
                </Show>
            </div>
        </div>
    }
}
