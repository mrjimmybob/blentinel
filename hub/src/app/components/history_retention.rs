//! "History" on the admin page (M6-01, D-111): how long raw results and each
//! summary are kept. The hub prunes by it every hour.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, nothing};
use crate::model::{HistoryRetention, Permission};

#[cfg(not(feature = "ssr"))]
const URL: &str = "/api/admin/history-retention";

#[component]
pub(crate) fn HistoryRetentionSection() -> impl IntoView {
    let me = Me::current();
    let can_edit = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());

    let retention: LocalResource<Result<HistoryRetention, String>> =
        LocalResource::new(move || {
            refresh.track();
            async move {
                #[cfg(not(feature = "ssr"))]
                {
                    fetch_json::<HistoryRetention>(URL).await
                }
                #[cfg(feature = "ssr")]
                {
                    Err(String::new())
                }
            }
        });

    let save = move |retention: HistoryRetention| {
        error.set(String::new());
        message.set(String::new());
        if let Err(e) = retention.validate() {
            error.set(format!("Not saved: {e}."));
            return;
        }
        let body = serde_json::to_string(&retention).unwrap_or_default();
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json(URL, &body).await {
                Ok(()) => {
                    message.set("Saved. It applies at the next hourly run.".into());
                    refresh.update(|n| *n += 1);
                }
                Err(e) => error.set(e),
            }
            #[cfg(feature = "ssr")]
            let _ = body;
        });
    };

    view! {
        <div class="admin-section history-retention">
            <h2>"History"</h2>
            <p class="info-text">
                "Each report also adds its results to summaries per 5 minutes, hour and day, which \
                 the uptime chart reads. Older detail is let go every hour; the summaries keep the \
                 history. Each period is 1 to "
                {HistoryRetention::MAX_DAYS.to_string()}
                " days, and a coarser summary is kept at least as long as a finer one."
            </p>
            {move || -> AnyView {
                match retention.get() {
                    Some(Ok(current)) => any(view! { <RetentionForm current can_edit on_save=save/> }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            {move || {
                let e = error.get();
                if e.is_empty() { nothing() } else { any(view! { <div class="error-message">{e}</div> }) }
            }}
            {move || {
                let m = message.get();
                if m.is_empty() { nothing() } else { any(view! { <div class="success-message">{m}</div> }) }
            }}
        </div>
    }
}

/// One tier's row: what it is, its days as typed, and a note.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn DaysRow(
    label: &'static str,
    name: &'static str,
    days: RwSignal<String>,
    can_edit: Signal<bool>,
    #[prop(optional)] note: &'static str,
) -> impl IntoView {
    #[cfg(not(feature = "ssr"))]
    let on_input = move |ev| days.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_input = |_| {};
    view! {
        <tr>
            <td>{label}</td>
            <td>
                <input type="number" name=name min="1" max=HistoryRetention::MAX_DAYS.to_string()
                    disabled=move || !can_edit.get()
                    prop:value=move || days.get() on:input=on_input/>
                " days"
            </td>
            <td class="info-text">{note}</td>
        </tr>
    }
}

/// The three periods as typed, and Save for those who may change them.
#[component]
fn RetentionForm(
    current: HistoryRetention,
    can_edit: Signal<bool>,
    on_save: impl Fn(HistoryRetention) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let raw = RwSignal::new(current.raw_days.to_string());
    let five_minute = RwSignal::new(current.five_minute_days.to_string());
    let hourly = RwSignal::new(current.hourly_days.to_string());
    // Anything that isn't a number reads as 0, which `validate` refuses.
    let days = |typed: RwSignal<String>| typed.get_untracked().trim().parse().unwrap_or(0);
    let save = move |_| {
        on_save(HistoryRetention {
            raw_days: days(raw),
            five_minute_days: days(five_minute),
            hourly_days: days(hourly),
        })
    };
    view! {
        <table class="admin-table history-retention-table">
            <thead>
                <tr><th></th><th>"Kept for"</th><th></th></tr>
            </thead>
            <tbody>
                <DaysRow label="Raw results" name="history_raw_days" days=raw can_edit
                    note="Each probe's newest report is always kept."/>
                <DaysRow label="5-minute summaries" name="history_five_minute_days" days=five_minute can_edit/>
                <DaysRow label="Hourly summaries" name="history_hourly_days" days=hourly can_edit/>
                <tr>
                    <td>"Daily summaries"</td>
                    <td>"Forever"</td>
                    <td></td>
                </tr>
            </tbody>
        </table>
        <Show when=move || can_edit.get()>
            <button class="btn btn-primary btn-sm" name="history_save" on:click=save>"Save"</button>
        </Show>
    }
}
