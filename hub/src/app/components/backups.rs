//! "Backups" on the admin page (M2-12, D-60): the daily backup's schedule,
//! "Back up now", and the backups in the backups folder.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::browser;
#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::model::BackupDownloadLink;
use crate::model::{BackupDownloadRequest, BackupFile, BackupOverview, BackupSchedule, Permission};

#[component]
pub(crate) fn BackupsSection() -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let overview: LocalResource<Result<BackupOverview, String>> = LocalResource::new(move || {
        refresh.track();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<BackupOverview>("/api/admin/backups").await
            }
            #[cfg(feature = "ssr")]
            {
                Err(String::new())
            }
        }
    });

    // POST `body` to `url`, then reload the section (or show the hub's reason).
    let act = move |url: String, body: String| {
        busy.set(true);
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            if let Err(e) = post_json(&url, &body).await {
                error.set(e);
            }
            #[cfg(feature = "ssr")]
            let _ = (url, body);
            busy.set(false);
            refresh.update(|n| *n += 1);
        });
    };
    let back_up_now = move |_| act("/api/admin/backups".into(), "{}".into());
    let save_schedule = move |schedule: BackupSchedule| {
        let body = serde_json::to_string(&schedule).unwrap_or_default();
        act("/api/admin/backups/schedule".into(), body);
    };
    let delete = move |name: String| {
        act(format!("/api/admin/backups/{name}/delete"), "{}".into());
    };

    view! {
        <div class="admin-section backups">
            <h2>"Backups"</h2>
            {move || -> AnyView {
                match overview.get() {
                    Some(Ok(data)) => any(view! {
                        <Status data=data.clone()/>
                        <Show when=move || can_manage.get()>
                            <ScheduleForm schedule=data.schedule busy=busy on_save=save_schedule error=error/>
                        </Show>
                        <BackupList backups=data.backups.clone() can_manage=can_manage busy=busy error=error on_delete=delete/>
                    }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            <Show when=move || can_manage.get()>
                <div class="backup-actions">
                    <button class="btn btn-primary" name="backup_now" disabled=busy on:click=back_up_now>
                        {move || if busy.get() { "Working…" } else { "Back up now" }}
                    </button>
                </div>
            </Show>
            {move || {
                let e = error.get();
                if e.is_empty() { nothing() } else { any(view! { <div class="error-message">{e}</div> }) }
            }}
        </div>
    }
}

/// The schedule in words, the next and last run, and where backups go.
#[component]
fn Status(data: BackupOverview) -> impl IntoView {
    let schedule = data.schedule;
    let when = if schedule.enabled {
        format!(
            "Daily at {:02}:00 UTC; the last {} kept.",
            schedule.hour_utc, schedule.keep
        )
    } else {
        "The daily backup is off.".to_string()
    };
    let next = data.next_at.map(|at| format!(" Next: {at}."));
    let last = data
        .last_run_at
        .as_ref()
        .filter(|_| data.last_error.is_none())
        .map(|at| format!(" Last: {at}."));
    let failed = data
        .last_run_at
        .as_ref()
        .zip(data.last_error.as_ref())
        .map(|(at, error)| {
            view! {
                <div class="storage-warning-banner backup-failed">
                    {format!("The scheduled backup of {at} failed: {error}")}
                </div>
            }
        });
    view! {
        <p class="backup-status">
            {when}{next}{last}
        </p>
        {failed}
        <p class="info-text">
            "Written to " <code>{data.folder}</code>
            " on the hub's server. Each backup holds the hub's private key, so downloading one asks \
             for your password. Keep copies somewhere safe, off this server, to survive losing it. Restore one with "
            <code>"hub restore <file>"</code> "."
        </p>
    }
}

/// On or off, the hour, how many to keep.
#[component]
fn ScheduleForm(
    schedule: BackupSchedule,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    on_save: impl Fn(BackupSchedule) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let enabled = RwSignal::new(schedule.enabled);
    let hour = RwSignal::new(schedule.hour_utc.to_string());
    let keep = RwSignal::new(schedule.keep.to_string());
    #[cfg(not(feature = "ssr"))]
    let (on_enabled, on_hour, on_keep) = (
        move |ev| enabled.set(event_target_checked(&ev)),
        move |ev| hour.set(input_value(&ev)),
        move |ev| keep.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_enabled, on_hour, on_keep) = (|_| {}, |_| {}, |_| {});

    let save = move |_| {
        let parsed = BackupSchedule {
            enabled: enabled.get(),
            hour_utc: hour.get().parse().unwrap_or(u8::MAX),
            keep: keep.get().trim().parse().unwrap_or(0),
        };
        match parsed.validate() {
            Ok(()) => on_save(parsed),
            Err(e) => error.set(format!("Not saved: {e}.")),
        }
    };
    let hours = (0..24u8)
        .map(|h| view! { <option value=h.to_string()>{format!("{h:02}:00 UTC")}</option> })
        .collect_view();

    view! {
        <div class="backup-schedule">
            <label>
                <input type="checkbox" name="backup_enabled" prop:checked=move || enabled.get() on:change=on_enabled/>
                " Daily backup at "
            </label>
            <select name="backup_hour" prop:value=move || hour.get() on:change=on_hour>
                {hours}
            </select>
            <label>
                ", keep the last "
                <input type="number" name="backup_keep" min="1" max=BackupSchedule::MAX_KEEP.to_string()
                    prop:value=move || keep.get() on:input=on_keep/>
            </label>
            <button class="btn btn-secondary btn-sm" name="backup_save" disabled=busy on:click=save>"Save"</button>
        </div>
    }
}

/// What a row is asking before it acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    Delete,
    /// The user's password, since a backup holds the hub's key (D-60).
    Password,
}

/// The backups, newest first: download (after the password) and delete
/// (after a confirmation).
#[component]
fn BackupList(
    backups: Vec<BackupFile>,
    can_manage: Signal<bool>,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    on_delete: impl Fn(String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    if backups.is_empty() {
        return any(view! { <div class="empty-state">"No backups yet."</div> });
    }
    let asking = RwSignal::new(None::<(String, Asking)>);
    let password = RwSignal::new(String::new());
    let download = move |name: String| {
        busy.set(true);
        error.set(String::new());
        let body = serde_json::to_string(&BackupDownloadRequest {
            password: password.get_untracked(),
        })
        .unwrap_or_default();
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_for::<BackupDownloadLink>(
                &format!("/api/admin/backups/{name}/download"),
                &body,
            )
            .await
            {
                // The answer is an attachment: the browser saves it and stays here.
                Ok(link) => {
                    asking.set(None);
                    browser::navigate(&link.url);
                }
                Err(e) => error.set(e),
            }
            #[cfg(feature = "ssr")]
            let _ = (name, body);
            password.set(String::new());
            busy.set(false);
        });
    };
    let rows = backups
        .into_iter()
        .map(|backup| {
            let name = backup.name.clone();
            let actions = {
                let name = name.clone();
                move || -> AnyView {
                    if !can_manage.get() {
                        return nothing();
                    }
                    let current = asking
                        .get()
                        .filter(|(n, _)| *n == name)
                        .map(|(_, what)| what);
                    let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
                    match current {
                        None => any(view! {
                            <button class="btn btn-secondary btn-sm" disabled=busy
                                on:click=move |_| asking.set(Some((n1.clone(), Asking::Password)))>"Download"</button>
                            " "
                            <button class="btn btn-danger btn-sm" disabled=busy
                                on:click=move |_| asking.set(Some((n2.clone(), Asking::Delete)))>"Delete"</button>
                        }),
                        Some(Asking::Delete) => any(view! {
                            "Delete for good? "
                            <button class="btn btn-danger btn-sm" disabled=busy on:click=move |_| {
                                asking.set(None);
                                on_delete(n3.clone());
                            }>"Yes"</button>
                            " "
                            <button class="btn btn-secondary btn-sm" on:click=move |_| asking.set(None)>"No"</button>
                        }),
                        Some(Asking::Password) => {
                            #[cfg(not(feature = "ssr"))]
                            let on_password = move |ev| password.set(input_value(&ev));
                            #[cfg(feature = "ssr")]
                            let on_password = |_| {};
                            any(view! {
                                <form class="backup-password" on:submit=move |ev| {
                                    ev.prevent_default();
                                    download(n3.clone());
                                }>
                                    <input type="password" name="backup_password" autocomplete="current-password"
                                        placeholder="Your password" prop:value=move || password.get() on:input=on_password/>
                                    " "
                                    <button type="submit" class="btn btn-primary btn-sm" name="backup_download" disabled=busy>"Download"</button>
                                    " "
                                    <button type="button" class="btn btn-secondary btn-sm" on:click=move |_| {
                                        password.set(String::new());
                                        asking.set(None);
                                    }>"Cancel"</button>
                                </form>
                            })
                        }
                    }
                }
            };
            let shown = name.clone();
            view! {
                <tr data-backup=name>
                    <td>{backup.created_at.clone()}</td>
                    <td>{backup.kind.label()}</td>
                    <td>{size(backup.size_bytes)}</td>
                    <td class="backup-name"><code>{shown}</code></td>
                    <td>{actions}</td>
                </tr>
            }
        })
        .collect_view();
    any(view! {
        <table class="admin-table backup-list">
            <thead>
                <tr>
                    <th>"Made"</th>
                    <th>"Kind"</th>
                    <th>"Size"</th>
                    <th>"File"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    })
}

/// A file size people read at a glance.
fn size(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}
