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
    DigestSettings, MailSecurity, NTFY_SAFE_TOPIC_LEN, Permission, RouteRule, RoutesRequest,
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

    // POST `body` to `url`, then reload (or show the hub's reason). `saved`
    // is set once the hub took it: that form then takes the hub's state from
    // the reload ([`follow`]).
    let act =
        move |url: String, body: String, done: &'static str, saved: Option<RwSignal<bool>>| {
            error.set(String::new());
            message.set(String::new());
            leptos::task::spawn_local(async move {
                #[cfg(not(feature = "ssr"))]
                match post_json(&url, &body).await {
                    Ok(()) => {
                        if let Some(saved) = saved {
                            saved.set(true);
                        }
                        message.set(done.into());
                        refresh.update(|n| *n += 1);
                    }
                    Err(e) => error.set(e),
                }
                #[cfg(feature = "ssr")]
                let _ = (url, body, done, saved);
            });
        };

    // The newest data from the hub. The sections are built once, from the
    // first, and read the later ones: a reload updates what they show and
    // never rebuilds a form, which emptied what was typed (CI #69).
    let latest = RwSignal::new(None::<AlertsOverview>);
    let load_error = RwSignal::new(None::<String>);
    Effect::new(move |_| match overview.get() {
        Some(Ok(data)) => {
            load_error.set(None);
            latest.set(Some(data));
        }
        Some(Err(e)) => load_error.set(Some(e)),
        None => {}
    });
    let loaded = Memo::new(move |_| latest.with(Option::is_some));

    view! {
        <h1 class="page-title">"Alerts"</h1>
        <p class="info-text">
            "Where alerts go. Channels are e-mail, a signed webhook, Teams, Slack, Discord, Telegram or ntfy (all optional); \
             the routing rules decide which alerts each one gets. Thresholds are on the admin and company pages."
        </p>
        <div class="login-error">{error}</div>
        <div class="form-message">{message}</div>
        {move || load_error.get().map(|e| view! { <div class="error">{format!("Error: {e}")}</div> })}
        // Tracks only `loaded`, which turns true once: built once.
        {move || -> AnyView {
            if !loaded.get() {
                return any(view! {
                    <Show when=move || load_error.with(Option::is_none)>
                        <div class="loading">"Loading…"</div>
                    </Show>
                });
            }
            let Some(first) = latest.get_untracked() else {
                return nothing();
            };
            let data = Signal::derive(move || latest.get().unwrap_or_else(|| first.clone()));
            any(view! {
                <Channels data can_edit error message act refresh/>
                <Rules data can_edit error act/>
                <DigestForm data can_edit error message act/>
                <Settings data can_edit error act/>
            })
        }}
    }
}

/// A form's save, as [`follow`] sees it: what it sent, and whether the hub
/// took it (`act` sets `taken`).
struct Saving<T: Send + Sync + 'static> {
    sent: StoredValue<Option<T>>,
    taken: RwSignal<bool>,
}

impl<T: Send + Sync + 'static> Clone for Saving<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Send + Sync + 'static> Copy for Saving<T> {}

impl<T: Send + Sync + 'static> Saving<T> {
    fn new() -> Self {
        Self {
            sent: StoredValue::new(None),
            taken: RwSignal::new(false),
        }
    }

    /// Sending `value`: the flag for `act` to set once the hub took it.
    fn start(self, value: T) -> RwSignal<bool> {
        self.sent.set_value(Some(value));
        self.taken.set(false);
        self.taken
    }
}

/// Keep a form in step with the hub without losing what is typed (CI #69).
/// The form remembers the hub's state it was last in step with: what it last
/// took, or what it sent once the hub took it. On each reload, it takes the
/// hub's state (`pick`, through `apply`) only if it still equals that, i.e.
/// nothing was typed since; otherwise its unsaved edits stay. So a form
/// nobody touched follows the hub (a removed channel removes its rule), and
/// a form being edited keeps the edits, whenever they were made.
fn follow<T>(
    data: Signal<AlertsOverview>,
    pick: impl Fn(&AlertsOverview) -> T + Send + Sync + 'static,
    form: impl Fn() -> T + Send + Sync + 'static,
    apply: impl Fn(T) + Send + Sync + 'static,
    saving: Saving<T>,
) where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    let base = StoredValue::new(data.with_untracked(&pick));
    Effect::new(move |_| {
        let now = data.with(&pick);
        if saving.taken.get_untracked() {
            saving.taken.set(false);
            if let Some(sent) = saving.sent.try_update_value(Option::take).flatten() {
                base.set_value(sent);
            }
        }
        if base.with_value(|b| untrack(&form) == *b) {
            apply(now.clone());
            base.set_value(now);
        }
    });
}

/// The channels, and a form to add or change one.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Channels(
    data: Signal<AlertsOverview>,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    message: RwSignal<String>,
    act: impl Fn(String, String, &'static str, Option<RwSignal<bool>>) + Copy + Send + Sync + 'static,
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
            None,
        );
        reset();
    };

    // The table follows the hub's data; the form below keeps its own state.
    let rows = move || {
        data.with(|d| d.channels.clone())
            .into_iter()
            .map(|c| {
                let id = c.id;
                let last = match (&c.last_delivery_at, &c.last_error) {
                    (None, _) => "never".to_string(),
                    (Some(at), None) => format!("{at}: delivered"),
                    (Some(at), Some(e)) => format!("{at}: failed: {e}"),
                };
                let failed = c.last_error.is_some();
                let (enabled, name, kind, target, secret_set) =
                    (c.enabled, c.name.clone(), c.kind.label(), c.target.clone(), c.secret_set);
                view! {
                    <tr class:inactive=!enabled>
                        <td>{name}</td>
                        <td>{kind}</td>
                        <td>{target}{secret_set.then_some(" (secret set)")}</td>
                        <td class:error-text=failed>{last}</td>
                        <td class="row-actions">
                            <Show when=move || can_edit.get()>
                                <button class="btn btn-secondary btn-sm" name="channel_test" on:click=move |_| test(id)>"Send test"</button>
                                <button class="btn btn-secondary btn-sm" name="channel_edit"
                                    on:click={ let c = c.clone(); move |_| edit(c.clone()) }>"Edit"</button>
                                <button class="btn btn-secondary btn-sm" name="channel_delete"
                                    on:click=move |_| act(format!("/api/admin/alerts/channels/{id}/delete"), "{}".into(), "Channel removed.", None)>
                                    "Remove"
                                </button>
                            </Show>
                        </td>
                    </tr>
                }
            })
            .collect_view()
    };

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
            {move || match data.with(|d| d.channels.is_empty()) {
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
                    // ntfy (D-70): on a server anyone can read, the topic is the password.
                    <Show when=move || kind.get() == ChannelKind::Ntfy>
                        <div class="ntfy-topic">
                            <button class="btn btn-secondary btn-sm" name="ntfy_random_topic"
                                on:click=move |_| target.update(|t| *t = random_topic(t))>"New random topic"</button>
                            <Show when=move || short_topic(&target.get())>
                                <span class="form-hint">
                                    "A short topic is easy to guess: anyone who guesses it can read these alerts, unless your server needs a token."
                                </span>
                            </Show>
                        </div>
                    </Show>
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

/// Is `url`'s ntfy topic short enough to guess (D-70)?
fn short_topic(url: &str) -> bool {
    crate::model::ntfy_parts(url).is_some_and(|(_, topic)| topic.len() < NTFY_SAFE_TOPIC_LEN)
}

/// `url` with a new random topic on its server (`https://ntfy.sh` when it
/// has none yet, as an example: any server works).
fn random_topic(url: &str) -> String {
    let server = match crate::model::ntfy_parts(url) {
        Some((server, _)) => server.to_string(),
        None => {
            let url = url.trim().trim_end_matches('/');
            if url.starts_with("https://") || url.starts_with("http://") {
                url.to_string()
            } else {
                "https://ntfy.sh".to_string()
            }
        }
    };
    format!("{server}/blentinel-{}", random_hex(8))
}

/// `n` random bytes as hex, from the browser's secure random source.
#[cfg(not(feature = "ssr"))]
fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    if let Some(crypto) = leptos::web_sys::window().and_then(|w| w.crypto().ok()) {
        let _ = crypto.get_random_values_with_u8_array(&mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(feature = "ssr")]
fn random_hex(n: usize) -> String {
    "0".repeat(n * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_topic_stays_on_its_server() {
        assert_eq!(
            random_topic(""),
            "https://ntfy.sh/blentinel-0000000000000000"
        );
        assert_eq!(
            random_topic("https://ntfy.example.com/old"),
            "https://ntfy.example.com/blentinel-0000000000000000"
        );
        assert_eq!(
            random_topic("https://ntfy.example.com/"),
            "https://ntfy.example.com/blentinel-0000000000000000"
        );
        assert!(short_topic("https://ntfy.sh/alerts"));
        assert!(!short_topic(&random_topic("")));
        assert!(
            !short_topic("not a url"),
            "no warning for what isn't a topic yet"
        );
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
    data: Signal<AlertsOverview>,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    act: impl Fn(String, String, &'static str, Option<RwSignal<bool>>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let rules = RwSignal::new(data.with_untracked(|d| d.rules.clone()));
    let saving = Saving::new();
    follow(
        data,
        |d| d.rules.clone(),
        move || rules.get_untracked(),
        move |r| rules.set(r),
        saving,
    );
    let save = move |_| {
        let request = RoutesRequest {
            rules: rules.get_untracked(),
        };
        let ids: Vec<i64> = data.with_untracked(|d| d.channels.iter().map(|c| c.id).collect());
        if let Err(e) = request.validate(&ids) {
            error.set(format!("Not saved: {e}."));
            return;
        }
        act(
            "/api/admin/alerts/routes".into(),
            serde_json::to_string(&request).unwrap_or_default(),
            "Rules saved.",
            Some(saving.start(request.rules)),
        );
    };
    let add = move |_| rules.update(|r| r.push(RouteRule::default()));

    let rule_views = move || {
        let d = data.get();
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

/// The daily digest (D-71): when, and to which e-mail channels.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn DigestForm(
    data: Signal<AlertsOverview>,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    message: RwSignal<String>,
    act: impl Fn(String, String, &'static str, Option<RwSignal<bool>>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let digest = data.with_untracked(|d| d.digest.clone());
    let hour = RwSignal::new(digest.hour.to_string());
    let timezone = RwSignal::new(digest.timezone);
    let chosen = RwSignal::new(digest.channels);
    // The form as the hub would hold it: what Save sends.
    let form = move || DigestSettings {
        hour: hour.get_untracked().trim().parse().unwrap_or(99),
        timezone: timezone.get_untracked().trim().to_string(),
        channels: chosen.get_untracked(),
    };
    let saving = Saving::new();
    follow(
        data,
        |d| d.digest.clone(),
        form,
        move |d| {
            hour.set(d.hour.to_string());
            timezone.set(d.timezone);
            chosen.set(d.channels);
        },
        saving,
    );
    let emails = move || -> Vec<ChannelInfo> {
        data.with(|d| {
            d.channels
                .iter()
                .filter(|c| c.kind == ChannelKind::Email)
                .cloned()
                .collect()
        })
    };
    let save = move |_| {
        let request = form();
        let email_ids: Vec<i64> = untrack(emails).iter().map(|c| c.id).collect();
        if let Err(e) = request.validate(&email_ids) {
            error.set(format!("Not saved: {e}."));
            return;
        }
        act(
            "/api/admin/alerts/digest".into(),
            serde_json::to_string(&request).unwrap_or_default(),
            "Digest saved.",
            Some(saving.start(request)),
        );
    };
    let send_now = move |_| {
        error.set(String::new());
        message.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_for::<TestResult>("/api/admin/alerts/digest/send", "{}").await {
                Ok(TestResult { ok: true, .. }) => message.set("Digest sent.".into()),
                Ok(TestResult { error: e, .. }) => {
                    error.set(format!("Not sent everywhere: {}", e.unwrap_or_default()))
                }
                Err(e) => error.set(e),
            }
        });
    };
    #[cfg(not(feature = "ssr"))]
    let set = |signal: RwSignal<String>| move |ev| signal.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let set = |_signal: RwSignal<String>| |_| {};
    let boxes = move || {
        emails()
        .into_iter()
        .map(|c| {
            let id = c.id;
            #[cfg(not(feature = "ssr"))]
            let on_change = move |ev| {
                let on = event_target_checked(&ev);
                chosen.update(|ids| {
                    ids.retain(|i| *i != id);
                    if on {
                        ids.push(id);
                    }
                });
            };
            #[cfg(feature = "ssr")]
            let on_change = |_| {};
            view! {
                <label class="rule-choice">
                    <input type="checkbox" name="digest_channel" prop:checked=move || chosen.with(|ids| ids.contains(&id))
                        on:change=on_change/>{c.name}
                </label>
            }
        })
        .collect_view()
    };
    let disabled = move || !can_edit.get();
    let last = move || {
        data.with(|d| d.digest_last_sent.clone())
            .map(|d| format!("Last sent for {d}."))
            .unwrap_or_else(|| "Not sent yet.".into())
    };
    view! {
        <div class="admin-section alerts-digest">
            <h2>"Daily digest"</h2>
            <p class="info-text">
                "Every day, one e-mail: what is open, what happened in the last 24 hours, which probes are silent, \
                 and each company's uptime. Also on a quiet day: if it stops coming, look at the hub. "{last}
            </p>
            <div class="settings-form">
                <label>"At "
                    <input type="number" name="digest_hour" min="0" max="23" size="3" disabled=disabled
                        prop:value=move || hour.get() on:input=set(hour)/>
                    ":00, in the time zone "
                    <input type="text" name="digest_timezone" placeholder="Europe/Madrid" disabled=disabled
                        prop:value=move || timezone.get() on:input=set(timezone)/>
                </label>
                <div class="rule-line"><span class="rule-label">"To (e-mail channels; none: off)"</span>{boxes}</div>
                <Show when=move || can_edit.get()>
                    <div class="form-actions">
                        <button class="btn btn-primary btn-sm" name="digest_save" on:click=save>"Save digest"</button>
                        <button class="btn btn-secondary btn-sm" name="digest_send" on:click=send_now>"Send digest now"</button>
                    </div>
                </Show>
            </div>
        </div>
    }
}

/// The hub's mail server and its address for links.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Settings(
    data: Signal<AlertsOverview>,
    can_edit: Signal<bool>,
    error: RwSignal<String>,
    act: impl Fn(String, String, &'static str, Option<RwSignal<bool>>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let settings = data.with_untracked(|d| d.settings.clone());
    let hub_url = RwSignal::new(settings.hub_url.clone());
    let server = RwSignal::new(settings.smtp_server.clone());
    let port = RwSignal::new(settings.smtp_port.to_string());
    let security = RwSignal::new(settings.smtp_security.as_str().to_string());
    let username = RwSignal::new(settings.smtp_username.clone());
    let from = RwSignal::new(settings.smtp_from.clone());
    let password = RwSignal::new(String::new());
    // The form as the hub would hold it: what Save sends. The password is
    // write-only: the hub never sends it back, so a typed one is an edit,
    // and taking the hub's state empties its box.
    let form = move || AlertSettings {
        hub_url: hub_url.get_untracked(),
        smtp_server: server.get_untracked(),
        smtp_port: port.get_untracked().trim().parse().unwrap_or(0),
        smtp_security: MailSecurity::parse(&security.get_untracked()).unwrap_or_default(),
        smtp_username: username.get_untracked(),
        smtp_from: from.get_untracked(),
        smtp_password: Some(password.get_untracked()).filter(|p| !p.is_empty()),
        smtp_password_set: false,
    };
    let saving = Saving::new();
    follow(
        data,
        |d| AlertSettings {
            smtp_password: None,
            smtp_password_set: false,
            ..d.settings.clone()
        },
        form,
        move |s| {
            hub_url.set(s.hub_url);
            server.set(s.smtp_server);
            port.set(s.smtp_port.to_string());
            security.set(s.smtp_security.as_str().to_string());
            username.set(s.smtp_username);
            from.set(s.smtp_from);
            password.set(String::new());
        },
        saving,
    );
    let password_set = move || data.with(|d| d.settings.smtp_password_set);
    #[cfg(not(feature = "ssr"))]
    let set = |signal: RwSignal<String>| move |ev| signal.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let set = |_signal: RwSignal<String>| |_| {};
    let save = move |_| {
        let request = form();
        if let Err(e) = request.validate() {
            error.set(format!("Not saved: {e}."));
            return;
        }
        act(
            "/api/admin/alerts/settings".into(),
            serde_json::to_string(&request).unwrap_or_default(),
            "Settings saved.",
            Some(saving.start(request)),
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
                        placeholder=move || if password_set() { "(set: leave empty to keep it)" } else { "" }
                        prop:value=move || password.get() on:input=set(password)/></label>
                <label>"From "<input type="email" name="smtp_from" placeholder="hub@example.com" disabled=disabled prop:value=move || from.get() on:input=set(from)/></label>
                <Show when=move || can_edit.get()>
                    <button class="btn btn-primary btn-sm" name="settings_save" on:click=save>"Save settings"</button>
                </Show>
            </div>
        </div>
    }
}
