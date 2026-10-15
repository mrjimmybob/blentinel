//! A company's maintenance page (M3-04, D-65): its maintenance windows
//! (planned work, one-off or recurring, in a time zone) and its silences
//! (from now), each for the whole company, a site, a probe or a check.
//! Changing them takes the `Silence` permission.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, fmt_ts};
use crate::model::{
    CreateSilenceRequest, MaintenanceOverview, MonthDay, NewWindowRequest, Permission, Schedule,
    Scope,
};

/// A scope as a `<select>` value: `company`, `site:ID`, `probe:ID`, `check:ID`.
fn scope_value(scope: &Scope) -> String {
    match scope.id() {
        Some(id) => format!("{}:{id}", scope.kind()),
        None => scope.kind().to_string(),
    }
}

fn scope_of(value: &str) -> Option<Scope> {
    match value.split_once(':') {
        Some((kind, id)) => Scope::from_parts(kind, Some(id.to_string())),
        None => Scope::from_parts(value, None),
    }
}

/// The scopes a company's windows and silences can cover, as `<option>`s.
fn scope_options(data: &MaintenanceOverview) -> Vec<(String, String)> {
    let mut options = vec![("company".to_string(), "The whole company".to_string())];
    for site in &data.sites {
        options.push((format!("site:{}", site.id), format!("Site: {}", site.name)));
    }
    for probe in &data.probes {
        options.push((
            format!("probe:{}", probe.probe_id),
            format!("Probe: {}", probe.probe_name),
        ));
        for (id, name) in &probe.checks {
            options.push((
                format!("check:{id}"),
                format!("Check: {name} ({})", probe.probe_name),
            ));
        }
    }
    options
}

/// The time zone a new window on `scope` starts with: its site's.
fn default_timezone(data: &MaintenanceOverview, scope: &str) -> String {
    let site_id = match scope_of(scope) {
        Some(Scope::Site { site_id }) => Some(site_id),
        Some(Scope::Probe { probe_id }) => data
            .probes
            .iter()
            .find(|p| p.probe_id == probe_id)
            .map(|p| p.site_id.clone()),
        Some(Scope::Check { check_id }) => data
            .probes
            .iter()
            .find(|p| p.checks.iter().any(|(id, _)| *id == check_id))
            .map(|p| p.site_id.clone()),
        _ => None,
    };
    site_id
        .and_then(|id| data.sites.iter().find(|s| s.id == id))
        .or_else(|| data.sites.first())
        .map_or_else(|| "UTC".to_string(), |s| s.timezone.clone())
}

/// The schedule the form describes, checked like the hub checks it.
#[allow(clippy::too_many_arguments)] // one form's fields
fn schedule_of(
    every: &str,
    once_start: &str,
    once_end: &str,
    days: &[u32],
    start: &str,
    hours: &str,
    by: &str,
    day: &str,
    week: &str,
    weekday: &str,
) -> Result<Schedule, String> {
    let minutes = || -> Result<u32, String> {
        let hours: f64 = hours
            .trim()
            .parse()
            .map_err(|_| format!("{hours}: a length in hours"))?;
        Ok((hours * 60.0).round() as u32)
    };
    let number = |text: &str| text.trim().parse::<u32>().unwrap_or(0);
    let schedule = match every {
        "once" => Schedule::Once {
            start: once_start.trim().to_string(),
            end: once_end.trim().to_string(),
        },
        "weekly" => Schedule::Weekly {
            days: days.to_vec(),
            start: start.trim().to_string(),
            minutes: minutes()?,
        },
        _ => Schedule::Monthly {
            on: match by {
                "date" => MonthDay::Date { day: number(day) },
                _ => MonthDay::Weekday {
                    week: number(week),
                    weekday: number(weekday),
                },
            },
            start: start.trim().to_string(),
            minutes: minutes()?,
        },
    };
    schedule.validate()?;
    Ok(schedule)
}

#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
pub(crate) fn MaintenancePage() -> impl IntoView {
    let params = use_params_map();
    let company_id = Signal::derive(move || params.get().get("company_id").unwrap_or_default());
    let me = Me::current();
    let can_change = Signal::derive(move || me.can(Permission::Silence));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());

    let overview: LocalResource<Result<MaintenanceOverview, String>> =
        LocalResource::new(move || {
            refresh.track();
            let id = company_id.get();
            async move {
                #[cfg(not(feature = "ssr"))]
                {
                    fetch_json::<MaintenanceOverview>(&format!("/api/companies/{id}/maintenance"))
                        .await
                }
                #[cfg(feature = "ssr")]
                {
                    let _ = id;
                    Err(String::new())
                }
            }
        });

    // POST `body` to `url`, then reload the page's data (or show the hub's reason).
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
        <div class="breadcrumb">
            <a href="/companies">"Companies"</a>
            <span class="sep">"›"</span>
            <a href=move || format!("/companies/{}", company_id.get())>{move || company_id.get()}</a>
            <span class="sep">"›"</span>
            "Maintenance"
        </div>
        <h1 class="page-title">"Maintenance and silences"</h1>
        <div class="login-error">{error}</div>
        <div class="form-message">{message}</div>
        {move || -> AnyView {
            match overview.get() {
                Some(Ok(data)) => any(view! {
                    <Windows data=data.clone() company_id=company_id.get() can_change error act/>
                    <Silences data company_id=company_id.get() can_change error act/>
                }),
                Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                None => any(view! { <div class="loading">"Loading…"</div> }),
            }
        }}
    }
}

/// One checkbox per weekday, into `days` (1 = Monday … 7 = Sunday).
fn day_boxes(days: RwSignal<Vec<u32>>) -> impl IntoView {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .into_iter()
        .zip(1u32..)
        .map(|(name, n)| {
            #[cfg(not(feature = "ssr"))]
            let on_day = move |ev| {
                let on = event_target_checked(&ev);
                days.update(|d| {
                    d.retain(|x| *x != n);
                    if on {
                        d.push(n);
                    }
                });
            };
            #[cfg(feature = "ssr")]
            let on_day = |_| {};
            view! {
                <label>
                    <input type="checkbox" name=format!("window_day_{n}")
                        prop:checked=move || days.with(|d| d.contains(&n)) on:change=on_day/>
                    {name}
                </label>
            }
        })
        .collect_view()
}

/// The company's maintenance windows, and a form for a new one.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Windows(
    data: MaintenanceOverview,
    company_id: String,
    can_change: Signal<bool>,
    error: RwSignal<String>,
    act: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let rows = data
        .windows
        .iter()
        .map(|w| {
            let id = w.window.id;
            let when = match w.active {
                true => any(view! { <span class="window-open">"Open now"</span> " · " {w.next.clone().unwrap_or_default()} }),
                false => any(view! { {w.next.clone().unwrap_or_else(|| "over".into())} }),
            };
            view! {
                <tr>
                    <td>{w.scope_label.clone()}</td>
                    <td>{w.schedule_text.clone()}" ("{w.window.timezone.clone()}")"</td>
                    <td>{w.window.reason.clone()}</td>
                    <td>{when}</td>
                    <td class="row-actions">
                        <Show when=move || can_change.get()>
                            <button class="btn btn-secondary btn-sm" name="window_delete"
                                on:click=move |_| act(format!("/api/maintenance/{id}/delete"), "{}".into(), "Window removed.")>
                                "Remove"
                            </button>
                        </Show>
                    </td>
                </tr>
            }
        })
        .collect_view();
    let options = StoredValue::new(scope_options(&data));
    let stored = StoredValue::new(data.clone());
    let company_id = StoredValue::new(company_id);

    // The form's fields.
    let scope = RwSignal::new("company".to_string());
    let reason = RwSignal::new(String::new());
    let timezone = RwSignal::new(default_timezone(&data, "company"));
    let every = RwSignal::new("weekly".to_string());
    let once_start = RwSignal::new(String::new());
    let once_end = RwSignal::new(String::new());
    let days = RwSignal::new(Vec::<u32>::new());
    let start = RwSignal::new("22:00".to_string());
    let hours = RwSignal::new("2".to_string());
    let by = RwSignal::new("weekday".to_string());
    let day = RwSignal::new("1".to_string());
    let week = RwSignal::new("2".to_string());
    let weekday = RwSignal::new("2".to_string());

    #[cfg(not(feature = "ssr"))]
    let set = |signal: RwSignal<String>| move |ev| signal.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let set = |_signal: RwSignal<String>| |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_scope = move |ev| {
        let value = input_value(&ev);
        timezone.set(default_timezone(&stored.get_value(), &value));
        scope.set(value);
    };
    #[cfg(feature = "ssr")]
    let on_scope = |_| {};

    let add = move |_| {
        error.set(String::new());
        let schedule = schedule_of(
            &every.get_untracked(),
            &once_start.get_untracked(),
            &once_end.get_untracked(),
            &days.get_untracked(),
            &start.get_untracked(),
            &hours.get_untracked(),
            &by.get_untracked(),
            &day.get_untracked(),
            &week.get_untracked(),
            &weekday.get_untracked(),
        );
        let request = schedule.and_then(|schedule| {
            let request = NewWindowRequest {
                scope: scope_of(&scope.get_untracked()).ok_or("choose what it covers")?,
                reason: reason.get_untracked(),
                timezone: timezone.get_untracked(),
                schedule,
            };
            request.validate()?;
            Ok(request)
        });
        match request {
            Ok(request) => act(
                format!("/api/companies/{}/maintenance", company_id.get_value()),
                serde_json::to_string(&request).unwrap_or_default(),
                "Window added.",
            ),
            Err(e) => error.set(format!("Not added: {e}.")),
        }
    };
    view! {
        <div class="admin-section maintenance">
            <h2>"Maintenance windows"</h2>
            <p class="info-text">
                "Planned work, once or on a schedule, in a time zone. While a window is open, what it covers sends no alerts, \
                 and its results don't count for uptime (the chart shades them). Whatever is still wrong when it closes \
                 alerts with the next report; a probe that stopped reporting during it, once it has closed."
            </p>
            {match data.windows.is_empty() {
                true => any(view! { <div class="empty-state">"No maintenance windows."</div> }),
                false => any(view! {
                    <table class="admin-table windows-table">
                        <thead><tr><th>"Covers"</th><th>"When"</th><th>"Why"</th><th>"Next"</th><th></th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                }),
            }}
            <Show when=move || can_change.get()>
                <div class="window-form">
                    <select name="window_scope" on:change=on_scope>
                        {options.get_value().into_iter().map(|(value, label)| view! { <option value=value>{label}</option> }).collect_view()}
                    </select>
                    <input type="text" name="window_reason" placeholder="Why, e.g. Patch Tuesday"
                        prop:value=move || reason.get() on:input=set(reason)/>
                    <select name="window_every" prop:value=move || every.get() on:change=set(every)>
                        <option value="once">"Once"</option>
                        <option value="weekly">"Every week"</option>
                        <option value="monthly">"Every month"</option>
                    </select>
                    <Show when=move || every.get() == "once">
                        <label>"from "<input type="datetime-local" name="window_once_start"
                            prop:value=move || once_start.get() on:input=set(once_start)/></label>
                        <label>"to "<input type="datetime-local" name="window_once_end"
                            prop:value=move || once_end.get() on:input=set(once_end)/></label>
                    </Show>
                    <Show when=move || every.get() == "weekly">
                        {day_boxes(days)}
                    </Show>
                    <Show when=move || every.get() == "monthly">
                        <select name="window_by" prop:value=move || by.get() on:change=set(by)>
                            <option value="weekday">"on the"</option>
                            <option value="date">"on day"</option>
                        </select>
                        <Show when=move || by.get() == "weekday"
                            fallback=move || view! {
                                <input type="number" name="window_day" min="1" max="31"
                                    prop:value=move || day.get() on:input=set(day)/>
                            }>
                            <select name="window_week" prop:value=move || week.get() on:change=set(week)>
                                <option value="1">"1st"</option><option value="2">"2nd"</option>
                                <option value="3">"3rd"</option><option value="4">"4th"</option>
                                <option value="5">"last"</option>
                            </select>
                            <select name="window_weekday" prop:value=move || weekday.get() on:change=set(weekday)>
                                <option value="1">"Monday"</option><option value="2">"Tuesday"</option>
                                <option value="3">"Wednesday"</option><option value="4">"Thursday"</option>
                                <option value="5">"Friday"</option><option value="6">"Saturday"</option>
                                <option value="7">"Sunday"</option>
                            </select>
                        </Show>
                    </Show>
                    <Show when=move || every.get() != "once">
                        <label>"at "<input type="time" name="window_start"
                            prop:value=move || start.get() on:input=set(start)/></label>
                        <label>"for "<input type="number" name="window_hours" min="0.25" step="0.25"
                            prop:value=move || hours.get() on:input=set(hours)/>" h"</label>
                    </Show>
                    <label>"time zone "<input type="text" name="window_timezone"
                        prop:value=move || timezone.get() on:input=set(timezone)/></label>
                    <button class="btn btn-primary btn-sm" name="window_add" on:click=add>"Add window"</button>
                </div>
            </Show>
        </div>
    }
}

/// The company's silences now, and a form for a new one.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn Silences(
    data: MaintenanceOverview,
    company_id: String,
    can_change: Signal<bool>,
    error: RwSignal<String>,
    act: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let options = scope_options(&data);
    let label_of = {
        let options = options.clone();
        move |scope: &crate::model::Scope| {
            let value = scope_value(scope);
            options
                .iter()
                .find(|(v, _)| *v == value)
                .map_or_else(|| "a deleted check".to_string(), |(_, l)| l.clone())
        }
    };
    let rows = data
        .silences
        .iter()
        .map(|s| {
            let id = s.id;
            let until = s
                .expires_at
                .as_deref()
                .map(fmt_ts)
                .unwrap_or_else(|| match s.scope {
                    Scope::Check { .. } => "it recovers".into(),
                    _ => "removed".into(),
                });
            view! {
                <tr>
                    <td>{label_of(&s.scope)}</td>
                    <td>{s.reason.clone()}</td>
                    <td>{until}</td>
                    <td class="row-actions">
                        <Show when=move || can_change.get()>
                            <button class="btn btn-secondary btn-sm" name="silence_delete" on:click=move |_| {
                                let body = serde_json::json!({ "silence_id": id }).to_string();
                                act("/api/silence/delete".into(), body, "Silence removed.");
                            }>"Remove"</button>
                        </Show>
                    </td>
                </tr>
            }
        })
        .collect_view();
    let scope = RwSignal::new("company".to_string());
    let reason = RwSignal::new(String::new());
    let hours = RwSignal::new("1".to_string());
    let company_id = StoredValue::new(company_id);
    let options = StoredValue::new(options);
    #[cfg(not(feature = "ssr"))]
    let set = |signal: RwSignal<String>| move |ev| signal.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let set = |_signal: RwSignal<String>| |_| {};
    let add = move |_| {
        error.set(String::new());
        let Some(scope) = scope_of(&scope.get_untracked()) else {
            return;
        };
        let request = CreateSilenceRequest {
            company_id: company_id.get_value(),
            scope,
            reason: reason.get_untracked(),
            duration_hours: hours.get_untracked().parse().ok(),
        };
        if request.reason.trim().is_empty() {
            error.set("Not silenced: say why.".into());
            return;
        }
        act(
            "/api/silence".into(),
            serde_json::to_string(&request).unwrap_or_default(),
            "Silenced.",
        );
    };

    view! {
        <div class="admin-section maintenance">
            <h2>"Silences"</h2>
            <p class="info-text">
                "Unplanned: from now, nothing alerts for what a silence covers. A check's silence also ends when it recovers."
            </p>
            {match data.silences.is_empty() {
                true => any(view! { <div class="empty-state">"Nothing is silenced."</div> }),
                false => any(view! {
                    <table class="admin-table silences-table">
                        <thead><tr><th>"Covers"</th><th>"Why"</th><th>"Until"</th><th></th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                }),
            }}
            <Show when=move || can_change.get()>
                <div class="silence-form">
                    <select name="silence_scope" on:change=set(scope)>
                        {options.get_value().into_iter().map(|(value, label)| view! { <option value=value>{label}</option> }).collect_view()}
                    </select>
                    <input type="text" name="silence_reason" placeholder="Why"
                        prop:value=move || reason.get() on:input=set(reason)/>
                    <select name="silence_hours" prop:value=move || hours.get() on:change=set(hours)>
                        <option value="1">"1 hour"</option>
                        <option value="24">"24 hours"</option>
                        <option value="168">"7 days"</option>
                        <option value="forever">"Until removed"</option>
                    </select>
                    <button class="btn btn-primary btn-sm" name="silence_add" on:click=add>"Silence"</button>
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_round_trip_through_their_select_value() {
        for scope in [
            Scope::Company,
            Scope::Site {
                site_id: "hq".into(),
            },
            Scope::Probe {
                probe_id: "p1".into(),
            },
            Scope::Check {
                check_id: "c:1".into(),
            },
        ] {
            assert_eq!(scope_of(&scope_value(&scope)), Some(scope));
        }
        assert_eq!(scope_of("nonsense"), None);
    }

    #[test]
    fn the_form_makes_a_schedule() {
        let weekly = schedule_of("weekly", "", "", &[2, 4], "22:00", "4", "", "", "", "").unwrap();
        assert_eq!(weekly.describe(), "every Tue, Thu at 22:00 for 4 h");
        let patch = schedule_of(
            "monthly",
            "",
            "",
            &[],
            "22:00",
            "1.5",
            "weekday",
            "",
            "2",
            "2",
        )
        .unwrap();
        assert_eq!(
            patch.describe(),
            "every month on the 2nd Tue at 22:00 for 1 h 30 min"
        );
        let once = schedule_of(
            "once",
            "2026-10-10T09:00",
            "2026-10-10T17:00",
            &[],
            "",
            "",
            "",
            "",
            "",
            "",
        );
        assert!(once.is_ok());
        assert!(
            schedule_of("weekly", "", "", &[], "22:00", "4", "", "", "", "").is_err(),
            "no days"
        );
        assert!(schedule_of("weekly", "", "", &[1], "22:00", "long", "", "", "", "").is_err());
    }
}
