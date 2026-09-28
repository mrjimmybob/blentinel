//! My account: your details, change your password (required after a
//! temporary one), and log out your other sessions.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, fmt_ts};
#[cfg(not(feature = "ssr"))]
use crate::app::{browser, http::post_json};
#[cfg(not(feature = "ssr"))]
use crate::model::ChangePasswordRequest;

#[component]
pub(crate) fn AccountPage() -> impl IntoView {
    let me = Me::current();
    let current = RwSignal::new(String::new());
    let new = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());

    let change_password = move || {
        message.set(String::new());
        if new.get() != confirm.get() {
            error.set("The new passwords don't match.".to_string());
            return;
        }
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = ChangePasswordRequest {
                    current_password: current.get_untracked(),
                    new_password: new.get_untracked(),
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_json("/api/me/password", &body).await {
                    // Reload so the layout sees the cleared "must change" flag.
                    Ok(()) if me.0.get_untracked().is_some_and(|u| u.must_change_password) => {
                        browser::navigate("/")
                    }
                    Ok(()) => {
                        for field in [current, new, confirm] {
                            field.set(String::new());
                        }
                        message
                            .set("Password changed. Your other sessions were logged out.".into());
                    }
                    Err(e) => error.set(e),
                }
            }
        });
    };

    let logout_others = move |_: _| {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json("/api/me/logout-others", "{}").await {
                Ok(()) => message.set("Your other sessions were logged out.".into()),
                Err(e) => error.set(e),
            }
        });
    };

    let field = move |signal: RwSignal<String>,
                      name: &'static str,
                      label: &'static str,
                      autocomplete: &'static str| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| signal.set(input_value(&ev));
        #[cfg(feature = "ssr")]
        let on_input = |_| {};
        view! {
            <div class="form-group">
                <label for=name>{label}</label>
                <input
                    id=name
                    type="password"
                    name=name
                    autocomplete=autocomplete
                    prop:value=move || signal.get()
                    on:input=on_input
                />
            </div>
        }
    };

    view! {
        <h1 class="page-title">"My account"</h1>
        <Show when=move || me.0.get().is_some_and(|u| u.must_change_password)>
            <div class="notice notice-warning">
                "You are using a temporary password. Choose your own to continue."
            </div>
        </Show>

        {move || me.0.get().map(|u| view! {
            <div class="admin-section">
                <h2>"Details"</h2>
                <table class="admin-table account-details">
                    <tr><th>"Username"</th><td>{u.username.clone()}</td></tr>
                    <tr><th>"Name"</th><td>{u.display_name.clone()}</td></tr>
                    <tr><th>"Role"</th><td>{u.role.label()}</td></tr>
                    <tr>
                        <th>"Last login"</th>
                        <td>{u.last_login_at.as_deref().map(fmt_ts).unwrap_or_else(|| "—".into())}</td>
                    </tr>
                </table>
            </div>
        })}

        <div class="admin-section">
            <h2>"Change password"</h2>
            <form class="form-card" on:submit=move |ev| {
                ev.prevent_default();
                change_password();
            }>
                {field(current, "current_password", "Current password", "current-password")}
                {field(new, "new_password", "New password (at least 12 characters)", "new-password")}
                {field(confirm, "confirm_password", "Repeat the new password", "new-password")}
                <button type="submit" class="btn btn-primary">"Change password"</button>
            </form>
        </div>

        <div class="admin-section">
            <h2>"Sessions"</h2>
            <p class="info-text">"Log out everywhere else, for example after using a shared computer."</p>
            <button class="btn btn-secondary" on:click=logout_others>"Log out other sessions"</button>
        </div>

        <div class="form-message">{message}</div>
        <div class="login-error">{error}</div>
    }
}
