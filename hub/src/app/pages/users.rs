//! Users (Owner, Admin): create users, change their role, deactivate them,
//! reset their password, log them out. The rules (who may manage Owners, the
//! last Owner, …) are the hub's (`service::users`); errors are shown as sent.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, fmt_ts};
#[cfg(not(feature = "ssr"))]
use crate::model::{CreateUserRequest, UpdateUserRequest};
use crate::model::{Role, TemporaryPassword, UserInfo};

/// Roles that can be given today: Client needs company scoping (M1-04), and
/// only an Owner can make Owners.
fn assignable_roles(me: Option<&UserInfo>) -> Vec<Role> {
    let is_owner = me.is_some_and(|u| u.role == Role::Owner);
    [Role::Owner, Role::Admin, Role::Technician, Role::Viewer]
        .into_iter()
        .filter(|r| is_owner || *r != Role::Owner)
        .collect()
}

#[component]
pub(crate) fn UsersPage() -> impl IntoView {
    let me = Me::current();
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    // A temporary password just created or reset, shown once.
    let issued = RwSignal::new(None::<TemporaryPassword>);
    let editing = RwSignal::new(None::<UserInfo>);

    let users: LocalResource<Result<Vec<UserInfo>, String>> = LocalResource::new(move || {
        refresh.track();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<UserInfo>>("/api/users").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        }
    });
    #[cfg_attr(feature = "ssr", allow(unused_variables))] // called by browser-only code
    let reload = move || refresh.update(|n| *n += 1);

    // --- create -----------------------------------------------------------
    let new_username = RwSignal::new(String::new());
    let new_display = RwSignal::new(String::new());
    let new_role = RwSignal::new(Role::Viewer);
    let create = move || {
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = CreateUserRequest {
                    username: new_username.get_untracked(),
                    display_name: new_display.get_untracked(),
                    email: None,
                    role: new_role.get_untracked(),
                    all_companies: true,
                    companies: vec![],
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<TemporaryPassword>("/api/users", &body).await {
                    Ok(created) => {
                        issued.set(Some(created));
                        new_username.set(String::new());
                        new_display.set(String::new());
                        reload();
                    }
                    Err(e) => error.set(e),
                }
            }
        });
    };

    // --- per-user actions -----------------------------------------------
    let reset_password = move |id: String| {
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_for::<TemporaryPassword>(&format!("/api/users/{id}/reset-password"), "{}")
                .await
            {
                Ok(reset) => issued.set(Some(reset)),
                Err(e) => error.set(e),
            }
            #[cfg(feature = "ssr")]
            let _ = id;
        });
    };
    let force_logout = move |id: String| {
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            if let Err(e) = post_json(&format!("/api/users/{id}/logout"), "{}").await {
                error.set(e);
            }
            #[cfg(feature = "ssr")]
            let _ = id;
        });
    };
    let save = move |user: UserInfo| {
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = UpdateUserRequest {
                    display_name: user.display_name,
                    email: user.email,
                    role: user.role,
                    all_companies: true,
                    companies: vec![],
                    is_active: user.is_active,
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<UserInfo>(&format!("/api/users/{}", user.id), &body).await {
                    Ok(_) => {
                        editing.set(None);
                        reload();
                    }
                    Err(e) => error.set(e),
                }
            }
            #[cfg(feature = "ssr")]
            let _ = user;
        });
    };

    #[cfg(not(feature = "ssr"))]
    let (on_username, on_display, on_role) = (
        move |ev| new_username.set(input_value(&ev)),
        move |ev| new_display.set(input_value(&ev)),
        move |ev| {
            if let Some(role) = Role::parse(&input_value(&ev)) {
                new_role.set(role);
            }
        },
    );
    #[cfg(feature = "ssr")]
    let (on_username, on_display, on_role) = (|_| {}, |_| {}, |_| {});

    view! {
        <h1 class="page-title">"Users"</h1>
        <div class="login-error">{error}</div>

        {move || issued.get().map(|t| view! {
            <div class="notice notice-info">
                <strong>{format!("Temporary password for {}: ", t.user.username)}</strong>
                <code class="temporary-password">{t.temporary_password}</code>
                <p>"It is shown only now. Give it to the user; they must change it when they log in."</p>
                <button class="btn btn-secondary btn-sm" on:click=move |_| issued.set(None)>"Done"</button>
            </div>
        })}

        <div class="admin-section">
            <h2>"All users"</h2>
            {move || -> AnyView {
                match users.get() {
                    Some(Ok(list)) => view! {
                        <table class="admin-table users-table">
                            <thead>
                                <tr>
                                    <th>"Username"</th><th>"Name"</th><th>"Role"</th>
                                    <th>"Status"</th><th>"Last login"</th><th></th>
                                </tr>
                            </thead>
                            <tbody>
                                {list.into_iter().map(|u| {
                                    let (id_reset, id_logout) = (u.id.clone(), u.id.clone());
                                    let for_edit = u.clone();
                                    let status = if !u.is_active {
                                        "Deactivated"
                                    } else if u.must_change_password {
                                        "Must change password"
                                    } else {
                                        "Active"
                                    };
                                    view! {
                                        <tr class:inactive=!u.is_active>
                                            <td>{u.username}</td>
                                            <td>{u.display_name}</td>
                                            <td>{u.role.label()}</td>
                                            <td>{status}</td>
                                            <td>{u.last_login_at.as_deref().map(fmt_ts).unwrap_or_else(|| "never".into())}</td>
                                            <td class="row-actions">
                                                <button class="btn btn-secondary btn-sm"
                                                    on:click=move |_| editing.set(Some(for_edit.clone()))>"Edit"</button>
                                                <button class="btn btn-warning btn-sm"
                                                    on:click=move |_| reset_password(id_reset.clone())>"Reset password"</button>
                                                <button class="btn btn-secondary btn-sm"
                                                    on:click=move |_| force_logout(id_logout.clone())>"Log out"</button>
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any(),
                    Some(Err(e)) => view! { <div class="error">{e}</div> }.into_any(),
                    None => view! { <div class="loading">"Loading…"</div> }.into_any(),
                }
            }}
        </div>

        <div class="admin-section">
            <h2>"Add a user"</h2>
            <form class="form-card form-inline" on:submit=move |ev| {
                ev.prevent_default();
                create();
            }>
                <input type="text" name="new_username" placeholder="Username"
                    prop:value=move || new_username.get() on:input=on_username/>
                <input type="text" name="new_display_name" placeholder="Display name (optional)"
                    prop:value=move || new_display.get() on:input=on_display/>
                <select name="new_role" on:change=on_role prop:value=move || new_role.get().as_str()>
                    {move || assignable_roles(me.0.get().as_ref()).into_iter().map(|r| view! {
                        <option value=r.as_str()>{r.label()}</option>
                    }).collect_view()}
                </select>
                <button type="submit" class="btn btn-primary">"Add user"</button>
            </form>
            <p class="info-text">
                "The hub creates a temporary password to hand over. Users limited to some companies, "
                "and the Client role, come with company scoping (ROADMAP M1-04)."
            </p>
        </div>

        {move || editing.get().map(|user| view! {
            <EditUserModal user=user editing=editing save=save/>
        })}
    }
}

/// Edit display name, role and active flag of one user.
#[component]
fn EditUserModal(
    user: UserInfo,
    editing: RwSignal<Option<UserInfo>>,
    save: impl Fn(UserInfo) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let me = Me::current();
    let draft = RwSignal::new(user.clone());
    #[cfg(not(feature = "ssr"))]
    let (on_display, on_role, on_active) = (
        move |ev| draft.update(|u| u.display_name = input_value(&ev)),
        move |ev| {
            if let Some(role) = Role::parse(&input_value(&ev)) {
                draft.update(|u| u.role = role);
            }
        },
        move |ev: leptos::ev::Event| {
            use leptos::wasm_bindgen::JsCast;
            let checked = ev
                .target()
                .and_then(|t| t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok())
                .is_some_and(|i| i.checked());
            draft.update(|u| u.is_active = checked);
        },
    );
    #[cfg(feature = "ssr")]
    let (on_display, on_role, on_active) = (|_| {}, |_| {}, |_| {});
    let mut roles = assignable_roles(me.0.get_untracked().as_ref());
    if !roles.contains(&user.role) {
        roles.insert(0, user.role); // shown, even if only an Owner may change it
    }

    view! {
        <div class="modal-overlay" on:click=move |_| editing.set(None)>
            <div class="modal" on:click=|e| e.stop_propagation()>
                <h3>{format!("Edit {}", user.username)}</h3>
                <div class="form-group">
                    <label>"Display name"</label>
                    <input type="text" name="edit_display_name"
                        prop:value=move || draft.get().display_name on:input=on_display/>
                </div>
                <div class="form-group">
                    <label>"Role"</label>
                    <select name="edit_role" on:change=on_role prop:value=move || draft.get().role.as_str()>
                        {roles.into_iter().map(|r| view! {
                            <option value=r.as_str()>{r.label()}</option>
                        }).collect_view()}
                    </select>
                </div>
                <div class="form-group">
                    <label>
                        <input type="checkbox" name="edit_active"
                            prop:checked=move || draft.get().is_active on:change=on_active/>
                        " Active (a deactivated user can't log in)"
                    </label>
                </div>
                <div class="modal-actions">
                    <button class="btn btn-secondary" on:click=move |_| editing.set(None)>"Cancel"</button>
                    <button class="btn btn-primary" on:click=move |_| save(draft.get_untracked())>"Save"</button>
                </div>
            </div>
        </div>
    }
}
