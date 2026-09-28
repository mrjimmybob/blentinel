//! Users (Owner, Admin): create users, choose their role and the companies
//! they see, deactivate them, reset their password, log them out. The rules
//! (who may manage Owners, the last Owner, valid scopes, …) are the hub's
//! (`service::users`); errors are shown as sent.

use std::collections::{BTreeSet, HashMap};

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, fmt_ts};
use crate::model::{CompanyInfo, CompanyScope, Role, TemporaryPassword, UserInfo};
#[cfg(not(feature = "ssr"))]
use crate::model::{CreateUserRequest, UpdateUserRequest};

/// Roles the current user may give: only an Owner makes Owners.
fn assignable_roles(me: Option<&UserInfo>) -> Vec<Role> {
    let is_owner = me.is_some_and(|u| u.role == Role::Owner);
    Role::ALL
        .into_iter()
        .filter(|r| is_owner || *r != Role::Owner)
        .collect()
}

/// A scope in words, with display names: "All", "ACME, Beta", "All except Zeta".
fn describe_scope(user: &UserInfo, names: &HashMap<String, String>) -> String {
    let list = |ids: &BTreeSet<String>| {
        ids.iter()
            .map(|id| names.get(id).cloned().unwrap_or_else(|| id.clone()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match user.effective_scope() {
        CompanyScope::All => "All".to_string(),
        CompanyScope::Only(ids) => list(&ids),
        CompanyScope::AllExcept(ids) => format!("All except {}", list(&ids)),
    }
}

/// The scope a role change leaves: a Client can only have "only these".
#[cfg_attr(feature = "ssr", allow(dead_code))] // called by browser-only code
fn scope_for_role(role: Role, scope: CompanyScope) -> CompanyScope {
    match (role, scope) {
        (Role::Client, CompanyScope::Only(ids)) => CompanyScope::Only(ids),
        (Role::Client, _) => CompanyScope::Only(BTreeSet::new()),
        (_, scope) => scope,
    }
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
    let companies_res: LocalResource<Result<Vec<CompanyInfo>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<CompanyInfo>>("/api/companies").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });
    let companies =
        Signal::derive(move || companies_res.get().and_then(Result::ok).unwrap_or_default());
    let names = Signal::derive(move || {
        companies
            .get()
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect::<HashMap<_, _>>()
    });
    #[cfg_attr(feature = "ssr", allow(unused_variables))] // called by browser-only code
    let reload = move || refresh.update(|n| *n += 1);

    // --- create -----------------------------------------------------------
    let new_username = RwSignal::new(String::new());
    let new_display = RwSignal::new(String::new());
    let new_role = RwSignal::new(Role::Viewer);
    let new_scope = RwSignal::new(CompanyScope::All);
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
                    scope: new_scope.get_untracked(),
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<TemporaryPassword>("/api/users", &body).await {
                    Ok(created) => {
                        issued.set(Some(created));
                        new_username.set(String::new());
                        new_display.set(String::new());
                        new_scope.set(scope_for_role(new_role.get_untracked(), CompanyScope::All));
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
                    scope: user.scope,
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
                new_scope.update(|s| *s = scope_for_role(role, std::mem::take(s)));
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
                                    <th>"Username"</th><th>"Name"</th><th>"Role"</th><th>"Companies"</th>
                                    <th>"Status"</th><th>"Last login"</th><th></th>
                                </tr>
                            </thead>
                            <tbody>
                                {list.into_iter().map(|u| {
                                    let (id_reset, id_logout) = (u.id.clone(), u.id.clone());
                                    let for_edit = u.clone();
                                    let scope = describe_scope(&u, &names.get());
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
                                            <td class="scope-cell">{scope}</td>
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
            <form class="form-card add-user-form" on:submit=move |ev| {
                ev.prevent_default();
                create();
            }>
                <div class="form-inline">
                    <input type="text" name="new_username" placeholder="Username"
                        prop:value=move || new_username.get() on:input=on_username/>
                    <input type="text" name="new_display_name" placeholder="Display name (optional)"
                        prop:value=move || new_display.get() on:input=on_display/>
                    <select name="new_role" on:change=on_role prop:value=move || new_role.get().as_str()>
                        {move || assignable_roles(me.0.get().as_ref()).into_iter().map(|r| view! {
                            <option value=r.as_str()>{r.label()}</option>
                        }).collect_view()}
                    </select>
                </div>
                <ScopePicker prefix="new" role=Signal::derive(move || new_role.get()) scope=new_scope companies=companies/>
                <button type="submit" class="btn btn-primary">"Add user"</button>
            </form>
            <p class="info-text">
                "The hub creates a temporary password to hand over. Owners and Admins see every company; "
                "the others see all companies, only the ones you tick, or all except the ones you tick "
                "(new companies included). A Client (a customer's login) sees only the companies you tick, "
                "without your internal notes and contacts."
            </p>
        </div>

        {move || editing.get().map(|user| view! {
            <EditUserModal user=user editing=editing save=save companies=companies/>
        })}
    }
}

/// Which companies a user sees: all, only the ticked ones, or all except
/// the ticked ones. Nothing to choose for Owner and Admin; a Client can only
/// have "only these".
#[component]
fn ScopePicker(
    /// Distinguishes the radio groups of several pickers on one page.
    prefix: &'static str,
    role: Signal<Role>,
    scope: RwSignal<CompanyScope>,
    companies: Signal<Vec<CompanyInfo>>,
) -> impl IntoView {
    let mode = Signal::derive(move || scope.get().mode());
    let set_mode = move |mode: &'static str| {
        scope.update(|s| {
            let ids = s.companies();
            *s = CompanyScope::from_parts(mode, ids).unwrap_or_default();
        });
    };
    let toggle = move |id: String| {
        scope.update(|s| {
            let mut ids: BTreeSet<String> = s.companies().into_iter().collect();
            if !ids.remove(&id) {
                ids.insert(id);
            }
            *s = CompanyScope::from_parts(s.mode(), ids).unwrap_or_default();
        });
    };
    let group = format!("{prefix}_scope");

    view! {
        <div class="scope-picker">
            {move || -> AnyView {
                if role.get().is_global() {
                    return view! { <p class="info-text">"Sees every company."</p> }.into_any();
                }
                let is_client = role.get() == Role::Client;
                let radio = |value: &'static str, label: &'static str| {
                    let group = group.clone();
                    view! {
                        <label class="scope-mode">
                            <input type="radio" name=group value=value
                                prop:checked=move || mode.get() == value
                                on:change=move |_| set_mode(value)/>
                            " " {label}
                        </label>
                    }
                };
                view! {
                    <div class="scope-modes">
                        {(!is_client).then(|| radio("all", "All companies"))}
                        {radio("only", "Only these")}
                        {(!is_client).then(|| radio("except", "All except these"))}
                    </div>
                    <Show when=move || mode.get() != "all">
                        <div class="scope-companies">
                            {move || companies.get().into_iter().map(|c| {
                                let id = c.id.clone();
                                let checked_id = c.id.clone();
                                view! {
                                    <label class="scope-company">
                                        <input type="checkbox" name=format!("{prefix}_company_{}", c.id)
                                            prop:checked=move || scope.get().companies().contains(&checked_id)
                                            on:change=move |_| toggle(id.clone())/>
                                        " " {c.name}
                                    </label>
                                }
                            }).collect_view()}
                        </div>
                    </Show>
                }.into_any()
            }}
        </div>
    }
}

/// Edit display name, role, companies and active flag of one user.
#[component]
fn EditUserModal(
    user: UserInfo,
    editing: RwSignal<Option<UserInfo>>,
    save: impl Fn(UserInfo) + Copy + Send + Sync + 'static,
    companies: Signal<Vec<CompanyInfo>>,
) -> impl IntoView {
    let me = Me::current();
    let draft = RwSignal::new(user.clone());
    let scope = RwSignal::new(user.scope.clone());
    #[cfg(not(feature = "ssr"))]
    let (on_display, on_role, on_active) = (
        move |ev| draft.update(|u| u.display_name = input_value(&ev)),
        move |ev| {
            if let Some(role) = Role::parse(&input_value(&ev)) {
                draft.update(|u| u.role = role);
                scope.update(|s| *s = scope_for_role(role, std::mem::take(s)));
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
                    <label>"Companies"</label>
                    <ScopePicker prefix="edit" role=Signal::derive(move || draft.get().role) scope=scope companies=companies/>
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
                    <button class="btn btn-primary" on:click=move |_| {
                        let mut user = draft.get_untracked();
                        user.scope = scope.get_untracked();
                        save(user);
                    }>"Save"</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_can_only_have_chosen_companies() {
        let only = CompanyScope::Only(["a".to_string()].into());
        assert_eq!(scope_for_role(Role::Client, only.clone()), only);
        assert_eq!(
            scope_for_role(Role::Client, CompanyScope::All),
            CompanyScope::Only(BTreeSet::new())
        );
        assert_eq!(
            scope_for_role(Role::Viewer, CompanyScope::All),
            CompanyScope::All
        );
    }
}
