//! Login page: exchanges the admin token for a session cookie.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
#[cfg(not(feature = "ssr"))]
use crate::app::{browser, http::post_text};

#[component]
pub(crate) fn LoginPage() -> impl IntoView {
    let token = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);

    let do_login = move || {
        let t = token.get();
        if t.trim().is_empty() {
            error.set("Please enter your token.".to_string());
            return;
        }

        submitting.set(true);
        error.set(String::new());

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                match post_text("/api/login", t.trim()).await {
                    Ok(200) => {
                        browser::navigate("/");
                    }
                    Ok(_) => {
                        error.set("Invalid token. Please try again.".to_string());
                    }
                    Err(e) => {
                        error.set(format!("Error: {}", e));
                    }
                }
            }
            submitting.set(false);
        });
    };

    let handle_click = move |_: _| {
        do_login();
    };
    #[cfg(not(feature = "ssr"))]
    let on_input = move |ev| {
        token.set(input_value(&ev));
    };
    #[cfg(feature = "ssr")]
    let on_input = |_| {};
    view! {
        <div class="login-page">
            <div class="login-card">
                <h1>"Blentinel Hub"</h1>
                <p>"Enter your admin token to access the dashboard."</p>
                <input
                    type="text"
                    placeholder="Admin token"
                    prop:value=move || token.get()
                    on:input=on_input
                />
                <button type="button" class="btn btn-primary" disabled=submitting on:click=handle_click>
                    {move || if submitting.get() { "Logging in…" } else { "Log In" }}
                </button>
                <div class="login-error">{error}</div>
            </div>
        </div>
    }
}
