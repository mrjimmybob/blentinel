//! First-run wizard: create the first Owner with the setup code the hub
//! printed to its log and wrote to `setup.code` (D-42).

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
#[cfg(not(feature = "ssr"))]
use crate::app::{browser, http::post_for};
#[cfg(not(feature = "ssr"))]
use crate::model::{SetupRequest, UserInfo};

#[component]
pub(crate) fn SetupPage() -> impl IntoView {
    let code = RwSignal::new(String::new());
    let username = RwSignal::new(String::new());
    let display_name = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);

    let submit = move || {
        if code.get().trim().is_empty() || username.get().trim().is_empty() {
            error.set("Enter the setup code and a username.".to_string());
            return;
        }
        if password.get() != confirm.get() {
            error.set("The passwords don't match.".to_string());
            return;
        }
        submitting.set(true);
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = SetupRequest {
                    code: code.get_untracked(),
                    username: username.get_untracked(),
                    display_name: display_name.get_untracked(),
                    password: password.get_untracked(),
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<UserInfo>("/api/setup", &body).await {
                    Ok(_) => browser::navigate("/"),
                    Err(e) => error.set(e),
                }
            }
            submitting.set(false);
        });
    };

    // One input bound to a signal (browser-side handler only).
    let field = move |signal: RwSignal<String>,
                      kind: &'static str,
                      name: &'static str,
                      placeholder: &'static str| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| signal.set(input_value(&ev));
        #[cfg(feature = "ssr")]
        let on_input = |_| {};
        view! {
            <input
                type=kind
                name=name
                placeholder=placeholder
                prop:value=move || signal.get()
                on:input=on_input
            />
        }
    };

    view! {
        <div class="login-page">
            <form class="login-card" on:submit=move |ev| {
                ev.prevent_default();
                submit();
            }>
                <h1>"Set up Blentinel"</h1>
                <p>
                    "Create the first Owner account. The setup code is in the hub's log and in "
                    <code>"setup.code"</code>" in its state folder."
                </p>
                {field(code, "text", "code", "Setup code")}
                {field(username, "text", "username", "Username")}
                {field(display_name, "text", "display_name", "Display name (optional)")}
                {field(password, "password", "password", "Password (at least 12 characters)")}
                {field(confirm, "password", "confirm", "Repeat the password")}
                <button type="submit" class="btn btn-primary" disabled=submitting>
                    {move || if submitting.get() { "Creating…" } else { "Create Owner" }}
                </button>
                <div class="login-error">{error}</div>
            </form>
        </div>
    }
}
