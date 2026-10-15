//! Login page: username and password for a session cookie. A hub without
//! users sends visitors to the setup wizard instead.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{fetch_json, post_for},
};
#[cfg(not(feature = "ssr"))]
use crate::model::{LoginRequest, SetupStatus, UserInfo};

#[component]
pub(crate) fn LoginPage() -> impl IntoView {
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);

    // First run: no users yet, so there is nobody to log in as.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        leptos::task::spawn_local(async move {
            if let Ok(SetupStatus { needed: true }) = fetch_json::<SetupStatus>("/api/setup").await
            {
                browser::navigate("/setup");
            }
        });
    });

    let do_login = move || {
        if username.get().trim().is_empty() || password.get().is_empty() {
            error.set("Enter your username and password.".to_string());
            return;
        }
        submitting.set(true);
        error.set(String::new());

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = LoginRequest {
                    username: username.get_untracked(),
                    password: password.get_untracked(),
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<UserInfo>("/api/auth/login", &body).await {
                    Ok(user) if user.must_change_password => browser::navigate("/account"),
                    Ok(_) => browser::navigate("/"),
                    Err(e) => error.set(e),
                }
            }
            submitting.set(false);
        });
    };

    #[cfg(not(feature = "ssr"))]
    let (on_username, on_password) = (
        move |ev| username.set(input_value(&ev)),
        move |ev| password.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_username, on_password) = (|_| {}, |_| {});

    view! {
        <div class="login-page">
            <form class="login-card" on:submit=move |ev| {
                ev.prevent_default();
                do_login();
            }>
                <h1>"Blentinel Hub"</h1>
                <p>"Log in to your account."</p>
                <input
                    type="text"
                    name="username"
                    autocomplete="username"
                    placeholder="Username"
                    prop:value=move || username.get()
                    on:input=on_username
                />
                <input
                    type="password"
                    name="password"
                    autocomplete="current-password"
                    placeholder="Password"
                    prop:value=move || password.get()
                    on:input=on_password
                />
                <button type="submit" class="btn btn-primary" disabled=submitting>
                    {move || if submitting.get() { "Logging in…" } else { "Log In" }}
                </button>
                <div class="login-error">{error}</div>
            </form>
        </div>
    }
}
