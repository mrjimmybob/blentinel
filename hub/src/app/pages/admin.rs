//! Admin page: storage, archives, backups, data deletion, probe removal,
//! probe updates, the hub's keys, hub info.

use leptos::prelude::*;

use crate::app::components::{BackupsSection, HubKeysSection, ProbeUpdatesSection};
use crate::app::ui::{any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{fetch_json, post_json},
};
use crate::model::{AdminProbe, ArchiveMetadata, StorageInfo};

/// The workspace version (Cargo.toml), so the page can't show a stale value.
const HUB_VERSION: &str = env!("CARGO_PKG_VERSION");

#[component]
pub(crate) fn AdminPage() -> impl IntoView {
    let companies: LocalResource<Result<Vec<String>, String>> = LocalResource::new(|| async move {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<Vec<String>>("/api/admin/companies").await
        }
        #[cfg(feature = "ssr")]
        {
            Ok(vec![])
        }
    });

    let probes: LocalResource<Result<Vec<AdminProbe>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<AdminProbe>>("/api/admin/probes").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    // Storage info resource
    let storage_info: LocalResource<Result<StorageInfo, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<StorageInfo>("/api/admin/storage-info").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(StorageInfo {
                    current_db_size_mb: 0,
                    warn_threshold_mb: 1000,
                    retention_days: 90,
                    last_archive_date: None,
                    last_archive_size_mb: None,
                    warning: false,
                })
            }
        });

    // Archives resource
    let archives: LocalResource<Result<Vec<ArchiveMetadata>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<ArchiveMetadata>>("/api/admin/archives").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    // Archive operation state
    let archiving = RwSignal::new(false);
    let archive_error = RwSignal::new(String::new());

    let do_archive = move |_: _| {
        archiving.set(true);
        archive_error.set(String::new());

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                match post_json("/api/admin/archive", "{}").await {
                    Ok(()) => {
                        storage_info.refetch();
                        archives.refetch();
                    }
                    Err(e) => {
                        archive_error.set(format!("Archive failed: {}", e));
                    }
                }
            }
            archiving.set(false);
        });
    };

    // Modal state
    let modal_title = RwSignal::new(String::new());
    let modal_message = RwSignal::new(String::new());
    let modal_action = RwSignal::new(String::new());
    let modal_url = RwSignal::new(String::new());
    let modal_open = RwSignal::new(false);

    let show_modal = move |title: &str, message: &str, url: &str, body: &str| {
        modal_title.set(title.to_string());
        modal_message.set(message.to_string());
        modal_url.set(url.to_string());
        modal_action.set(body.to_string());
        modal_open.set(true);
    };

    let confirm_action = move |_: _| {
        let _url = modal_url.get();
        let _body = modal_action.get();
        modal_open.set(false);

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                match post_json(&_url, &_body).await {
                    Ok(()) => {
                        browser::reload();
                    }
                    Err(e) => {
                        leptos::logging::error!("Admin action failed: {}", e);
                    }
                }
            }
        });
    };

    let cancel_modal = move |_: _| {
        modal_open.set(false);
    };

    view! {
        <Suspense fallback=|| view! { <div class="loading">"Loading…"</div> }>
            // Storage & Retention Section
            <div class="admin-section">
                <h2>"Storage & Retention"</h2>
                {move || -> AnyView {
                    match storage_info.get() {
                        Some(Ok(info)) => {
                            let warning_class = if info.warning { "storage-warning" } else { "" };
                            let last_archive_date_display = info.last_archive_date
                                .as_deref()
                                .unwrap_or("Never")
                                .to_string();
                            let last_archive_size_display = info.last_archive_size_mb
                                .map(|s| format!("{} MB", s))
                                .unwrap_or_else(|| "—".to_string());

                            any(view! {
                                <div class={format!("storage-info-card {}", warning_class)}>
                                    <div class="storage-stats">
                                        <div class="storage-stat">
                                            <span class="stat-label">"Current DB Size:"</span>
                                            <span class="stat-value">{format!("{} MB", info.current_db_size_mb)}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Warning Threshold:"</span>
                                            <span class="stat-value">{format!("{} MB", info.warn_threshold_mb)}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Retention Window:"</span>
                                            <span class="stat-value">{format!("{} days", info.retention_days)}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Last Archive Date:"</span>
                                            <span class="stat-value">{last_archive_date_display}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Last Archive Size:"</span>
                                            <span class="stat-value">{last_archive_size_display}</span>
                                        </div>
                                    </div>

                                    {move || if info.warning {
                                        any(view! {
                                            <div class="storage-warning-banner">
                                                "⚠ Database size exceeds threshold. Consider archiving old data."
                                            </div>
                                        })
                                    } else {
                                        nothing()
                                    }}

                                    <div class="storage-actions">
                                        <button
                                            class="btn btn-primary"
                                            disabled=archiving
                                            on:click=do_archive
                                        >
                                            {move || if archiving.get() { "Archiving…" } else { "Archive Old Data Now" }}
                                        </button>
                                    </div>

                                    {move || {
                                        let err = archive_error.get();
                                        if !err.is_empty() {
                                            any(view! {
                                                <div class="error-message">{err}</div>
                                            })
                                        } else {
                                            nothing()
                                        }
                                    }}
                                </div>
                            })
                        }
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            // Archives Section
            <div class="admin-section">
                <h2>"Archives"</h2>
                {move || -> AnyView {
                    match archives.get() {
                        Some(Ok(list)) if !list.is_empty() => {
                            any(view! {
                                <table class="admin-table">
                                    <thead>
                                        <tr>
                                            <th>"Date Created"</th>
                                            <th>"Data Cutoff"</th>
                                            <th>"Size (MB)"</th>
                                            <th>"Actions"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        <For
                                            each=move || list.clone()
                                            key=|a: &ArchiveMetadata| a.id
                                            children=|archive: ArchiveMetadata| {
                                                let archive_url = format!("/archive/{}", archive.id);
                                                view! {
                                                    <tr>
                                                        <td>{archive.created_at.clone()}</td>
                                                        <td>{archive.cutoff_date.clone()}</td>
                                                        <td>{archive.size_mb}</td>
                                                        <td>
                                                            <a href=archive_url class="btn btn-primary btn-sm">"Open"</a>
                                                        </td>
                                                    </tr>
                                                }
                                            }
                                        />
                                    </tbody>
                                </table>
                            })
                        }
                        Some(Ok(_)) => any(view! {
                            <div class="empty-state">"No archives yet. Archive old data to create an archive."</div>
                        }),
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            <BackupsSection/>

            <div class="admin-section">
                <h2>"Companies"</h2>
                {move || -> AnyView {
                    match companies.get() {
                        Some(Ok(list)) if !list.is_empty() => {
                            let show = show_modal;
                            any(view! {
                                <div class="admin-list">
                                    <For
                                        each=move || list.clone()
                                        key=|c: &String| c.clone()
                                        children=move |company: String| {
                                            let c = company.clone();
                                            let on_delete = move |_: _| {
                                                show(
                                                    "Delete Company Data",
                                                    &format!("Delete all report data for \"{}\"? The probe heartbeat will remain.", c),
                                                    "/api/admin/delete-company-data",
                                                    &format!("{{\"company_id\":\"{}\"}}", c),
                                                );
                                            };
                                            view! {
                                                <div class="admin-list-item">
                                                    <div class="admin-item-info">
                                                        <span class="admin-item-name">{company.clone()}</span>
                                                    </div>
                                                    <div class="admin-item-actions">
                                                        <button class="btn btn-danger" on:click=on_delete>"Delete Data"</button>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    />
                                </div>
                            })
                        }
                        Some(Ok(_)) => any(view! { <div class="empty-state">"No companies."</div> }),
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            <div class="admin-section">
                <h2>"Probes"</h2>
                {move || -> AnyView {
                    match probes.get() {
                        Some(Ok(list)) if !list.is_empty() => {
                            let show = show_modal;
                            any(view! {
                                <div class="admin-list">
                                    <For
                                        each=move || list.clone()
                                        key=|p: &AdminProbe| p.probe_id.clone()
                                        children=move |probe: AdminProbe| {
                                            let pid_del = probe.probe_id.clone();
                                            let pid_rem = probe.probe_id.clone();
                                            let cid = probe.company_id.clone();
                                            let short_id = format!("{}…", crate::model::short_probe_id(&probe.probe_id));
                                            let short_id2 = short_id.clone();
                                            let short_id_label = short_id.clone();

                                            let on_delete_data = move |_: _| {
                                                show(
                                                    "Delete Probe Data",
                                                    &format!("Delete all report data for probe \"{}\"? The heartbeat will remain.", short_id),
                                                    "/api/admin/delete-probe-data",
                                                    &format!("{{\"probe_id\":\"{}\"}}", pid_del),
                                                );
                                            };
                                            let on_remove = move |_: _| {
                                                show(
                                                    "Remove Probe",
                                                    &format!("Completely remove probe \"{}\"? This deletes all data and the heartbeat.", short_id2),
                                                    "/api/admin/remove-probe",
                                                    &format!("{{\"probe_id\":\"{}\"}}", pid_rem),
                                                );
                                            };

                                            view! {
                                                <div class="admin-list-item">
                                                    <div class="admin-item-info">
                                                        <span class="admin-item-name">{short_id_label}</span>
                                                        <span class="admin-item-sub">{format!("Company: {}", cid)}</span>
                                                    </div>
                                                    <div class="admin-item-actions">
                                                        <button class="btn btn-warning" on:click=on_delete_data>"Delete Data"</button>
                                                        <button class="btn btn-danger" on:click=on_remove>"Remove"</button>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    />
                                </div>
                            })
                        }
                        Some(Ok(_)) => any(view! { <div class="empty-state">"No probes."</div> }),
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            <ProbeUpdatesSection/>

            <HubKeysSection/>

            <div class="admin-section">
                <h2>"Hub Version"</h2>

                <div class="storage-info-card">
                    <div class="storage-stats">
                        <div class="storage-stat">
                            <span class="stat-label">"Version:"</span>
                            <span class="stat-value">{HUB_VERSION}</span>
                        </div>
                    </div>
                </div>
            </div>
        </Suspense>

        // Confirmation modal
        {move || -> AnyView {
            if modal_open.get() {
                any(view! {
                    <div class="modal-overlay">
                        <div class="modal">
                            <h3>{modal_title.get()}</h3>
                            <p>{modal_message.get()}</p>
                            <div class="modal-actions">
                                <button class="btn btn-secondary" on:click=cancel_modal>"Cancel"</button>
                                <button class="btn btn-danger" on:click=confirm_action>"Confirm"</button>
                            </div>
                        </div>
                    </div>
                })
            } else {
                nothing()
            }
        }}
    }
}
