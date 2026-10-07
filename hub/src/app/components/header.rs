//! Top bar: navigation (by permission), who is logged in, theme toggle, logout.

use leptos::prelude::*;

use crate::app::ui::{Me, any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{fetch_json, post_json},
};
use crate::model::{Permission, StorageInfo};

#[component]
pub(crate) fn Header() -> impl IntoView {
    let theme = RwSignal::new("light".to_string());
    let me = Me::current();

    // DB size, for users who may see the admin page (fetched once they're known).
    let db_size_info: LocalResource<Result<StorageInfo, String>> = LocalResource::new(move || {
        let allowed = me.can(Permission::ViewAdmin);
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                if !allowed {
                    return Err(String::new());
                }
                fetch_json::<StorageInfo>("/api/admin/storage-info").await
            }
            #[cfg(feature = "ssr")]
            {
                let _ = allowed; // local resources only load in the browser
                Err(String::new())
            }
        }
    });

    // On client mount: read persisted theme preference
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        if let Some(saved) = browser::stored_theme() {
            browser::apply_theme(&saved);
            theme.set(saved);
        }
    });

    let toggle_theme = move |_: _| {
        let next = if theme.get() == "dark" {
            "light"
        } else {
            "dark"
        };
        theme.set(next.to_string());
        #[cfg(not(feature = "ssr"))]
        {
            browser::apply_theme(next);
            browser::store_theme(next);
        }
    };

    let handle_logout = move |_: _| {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let _ = post_json("/api/auth/logout", "{}").await;
                browser::navigate("/login");
            }
        });
    };

    view! {
        <header class="header">
            <div class="header-left">
                <img class="logo-img" src="/logo.png" alt="Blentinel"
                     onerror="this.style.display='none'"/>
                <span class="logo-text">"Blentinel"</span>
                <ul class="nav-links">
                    <li><a href="/">"Dashboard"</a></li>
                    <li><a href="/companies">"Companies"</a></li>
                    <Show when=move || me.can(Permission::ViewAdmin)>
                        <li><a href="/admin">"Admin"</a></li>
                        <li><a href="/alerts">"Alerts"</a></li>
                    </Show>
                    <Show when=move || me.can(Permission::ManageUsers)>
                        <li><a href="/users">"Users"</a></li>
                    </Show>
                    <Show when=move || me.can(Permission::ViewAdmin)>
                        <li><a href="/audit">"Audit"</a></li>
                    </Show>
                </ul>
            </div>
            <div class="header-right">
                // DB size indicator
                {move || -> AnyView {
                    match db_size_info.get() {
                        Some(Ok(info)) => {
                            let size_class = if info.warning { "db-size-warning" } else { "db-size-normal" };
                            let tooltip = if info.warning {
                                "Database exceeds retention threshold"
                            } else {
                                "Current database size"
                            };
                            any(view! {
                                <span class={format!("db-size-indicator {}", size_class)} title=tooltip>
                                    {format!("DB: {} MB", info.current_db_size_mb)}
                                </span>
                            })
                        }
                        Some(Err(_)) => nothing(),
                        None => nothing(),
                    }
                }}
                <button class="btn btn-ghost" on:click=toggle_theme>
                    {move || if theme.get() == "dark" { "☀️ Light" } else { "🌙 Dark" }}
                </button>
                {move || me.0.get().map(|u| view! {
                    <a class="header-user" href="/account" title="My account">
                        {u.display_name}
                        <span class="header-role">{u.role.label()}</span>
                    </a>
                })}
                <button class="btn btn-ghost" on:click=handle_logout>"Logout"</button>
            </div>
        </header>
    }
}
