//! Audit log (Owner, Admin): who did what, when, from where (D-46).
//! Filters, newest first, "Load older" pages, and CSV export.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::fetch_json;
use crate::app::ui::fmt_ts;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::model::AuditEntry;

/// The action groups (the filter matches a prefix).
const ACTION_GROUPS: [(&str, &str); 11] = [
    ("", "All actions"),
    ("login", "Logins"),
    ("setup", "Setup"),
    ("account", "Own account"),
    ("user", "Users"),
    ("company", "Companies"),
    ("site", "Sites"),
    ("silence", "Silences"),
    ("data", "Data deletion"),
    ("probe", "Probes"),
    ("archive", "Archives"),
];

#[cfg_attr(feature = "ssr", allow(dead_code))] // used by browser-only code
const PAGE: u32 = 100;

/// `?user=…&action=…` from the filter values (empty ones left out). Dates are
/// whole days in UTC; "to" includes its day.
fn query_string(user: &str, action: &str, target: &str, from: &str, to: &str) -> String {
    let mut parts = Vec::new();
    let mut add = |key: &str, value: String| {
        if !value.is_empty() {
            parts.push(format!("{key}={}", encode(&value)));
        }
    };
    add("user", user.trim().to_string());
    add("action", action.to_string());
    add("target", target.trim().to_string());
    if !from.is_empty() {
        add("from", format!("{from}T00:00:00Z"));
    }
    if !to.is_empty() {
        add("to", format!("{to}T23:59:59Z"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Percent-encode a query value (enough for usernames, IDs and times).
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[component]
pub(crate) fn AuditPage() -> impl IntoView {
    let user = RwSignal::new(String::new());
    let action = RwSignal::new(String::new());
    let target = RwSignal::new(String::new());
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());
    let entries = RwSignal::new(Vec::<AuditEntry>::new());
    let more = RwSignal::new(false);
    let error = RwSignal::new(String::new());

    let query = Signal::derive(move || {
        query_string(
            &user.get(),
            &action.get(),
            &target.get(),
            &from.get(),
            &to.get(),
        )
    });

    // Load the first page, or (with `older`) the page after the last entry shown.
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let load = move |older: bool| {
        let base = query.get_untracked();
        let before = older
            .then(|| entries.with_untracked(|e| e.last().map(|e| e.id)))
            .flatten();
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let sep = if base.is_empty() { "?" } else { "&" };
                let mut url = format!("/api/audit{base}{sep}limit={PAGE}");
                if let Some(id) = before {
                    url.push_str(&format!("&before_id={id}"));
                }
                match fetch_json::<Vec<AuditEntry>>(&url).await {
                    Ok(page) => {
                        error.set(String::new());
                        more.set(page.len() as u32 == PAGE);
                        if older {
                            entries.update(|e| e.extend(page));
                        } else {
                            entries.set(page);
                        }
                    }
                    Err(e) => error.set(e),
                }
            }
            #[cfg(feature = "ssr")]
            let _ = (base, before);
        });
    };
    // Reload whenever a filter changes.
    Effect::new(move |_| {
        query.track();
        load(false);
    });

    let text_filter =
        move |signal: RwSignal<String>, name: &'static str, placeholder: &'static str| {
            #[cfg(not(feature = "ssr"))]
            let on_change = move |ev| signal.set(input_value(&ev));
            #[cfg(feature = "ssr")]
            let on_change = |_| {};
            view! {
                <input type="text" name=name placeholder=placeholder
                    prop:value=move || signal.get() on:change=on_change/>
            }
        };
    let date_filter = move |signal: RwSignal<String>, name: &'static str, label: &'static str| {
        #[cfg(not(feature = "ssr"))]
        let on_change = move |ev| signal.set(input_value(&ev));
        #[cfg(feature = "ssr")]
        let on_change = |_| {};
        view! {
            <label class="audit-date">{label}" "
                <input type="date" name=name prop:value=move || signal.get() on:change=on_change/>
            </label>
        }
    };
    #[cfg(not(feature = "ssr"))]
    let on_action = move |ev| action.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_action = |_| {};

    view! {
        <h1 class="page-title">"Audit log"</h1>
        <p class="info-text">"Who did what, newest first. Times are UTC."</p>
        <div class="form-card form-inline audit-filters">
            {text_filter(user, "audit_user", "Username")}
            <select name="audit_action" on:change=on_action prop:value=move || action.get()>
                {ACTION_GROUPS.iter().map(|(value, label)| view! {
                    <option value=*value>{*label}</option>
                }).collect_view()}
            </select>
            {text_filter(target, "audit_target", "Company, user or probe ID")}
            {date_filter(from, "audit_from", "From")}
            {date_filter(to, "audit_to", "To")}
            <a class="btn btn-secondary" href=move || format!("/api/audit.csv{}", query.get())>"Export CSV"</a>
        </div>
        <div class="login-error">{error}</div>
        <Show when=move || entries.with(Vec::is_empty) && error.get().is_empty()>
            <div class="empty-state">"No entries."</div>
        </Show>
        <Show when=move || !entries.with(Vec::is_empty)>
            <table class="admin-table audit-table">
                <thead>
                    <tr>
                        <th>"When"</th><th>"Who"</th><th>"From"</th><th>"Action"</th>
                        <th>"Target"</th><th>"Company"</th><th>"Details"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || entries.get().into_iter().map(|e| {
                        let target = match (&e.target_type, &e.target_id) {
                            (Some(kind), Some(id)) => format!("{kind} {id}"),
                            (_, Some(id)) => id.clone(),
                            _ => String::new(),
                        };
                        view! {
                            <tr>
                                <td class="nowrap">{fmt_ts(&e.at)}</td>
                                <td>{e.username.unwrap_or_else(|| "hub".into())}</td>
                                <td>{e.actor_ip.unwrap_or_default()}</td>
                                <td><code>{e.action}</code></td>
                                <td>{target}</td>
                                <td>{e.company_id.unwrap_or_default()}</td>
                                <td class="audit-details">{e.details.unwrap_or_default()}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
            <Show when=move || more.get()>
                <button class="btn btn-secondary" on:click=move |_| load(true)>"Load older"</button>
            </Show>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_become_a_query_string() {
        assert_eq!(query_string("", "", "", "", ""), "");
        assert_eq!(
            query_string(" alice ", "user", "", "2026-09-01", "2026-09-30"),
            "?user=alice&action=user&from=2026-09-01T00%3A00%3A00Z&to=2026-09-30T23%3A59%3A59Z"
        );
        assert_eq!(encode("a b&c"), "a%20b%26c");
    }
}
