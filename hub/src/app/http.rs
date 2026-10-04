//! Browser-side calls to the hub's JSON API (compiled for the client only).

use gloo_net::http::Request;

use crate::model::AlertSilence;

/// The error `fetch_json` returns for 401: the session is gone, go to /login.
pub(crate) const UNAUTHORIZED: &str = "UNAUTHORIZED";

/// GET `url` and decode its JSON. Errors are the hub's message (or
/// [`UNAUTHORIZED`]), ready to show.
pub(crate) async fn fetch_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = Request::get(url)
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if resp.status() == 401 {
        return Err(UNAUTHORIZED.to_string());
    }
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("JSON error: {}", e))
}

/// POST a JSON body; `Err` carries the hub's message.
pub(crate) async fn post_json(url: &str, body: &str) -> Result<(), String> {
    let resp = Request::post(url)
        .header("Content-Type", "application/json")
        .header("X-Blentinel", "1") // CSRF check (D-43)
        .body(body)
        .map_err(|e| format!("Body error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if resp.ok() {
        Ok(())
    } else {
        Err(error_message(resp).await)
    }
}

/// POST a JSON body and decode the JSON answer; `Err` carries the hub's message.
pub(crate) async fn post_for<T: serde::de::DeserializeOwned>(
    url: &str,
    body: &str,
) -> Result<T, String> {
    let resp = Request::post(url)
        .header("Content-Type", "application/json")
        .header("X-Blentinel", "1") // CSRF check (D-43)
        .body(body)
        .map_err(|e| format!("Body error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("JSON error: {}", e))
}

/// POST a file the user chose as the raw body (a probe release, D-55) and
/// decode the JSON answer; `Err` carries the hub's message.
pub(crate) async fn post_file<T: serde::de::DeserializeOwned>(
    url: &str,
    file: leptos::web_sys::File,
) -> Result<T, String> {
    let resp = Request::post(url)
        .header("Content-Type", "application/gzip")
        .header("X-Blentinel", "1") // CSRF check (D-43)
        .body(leptos::wasm_bindgen::JsValue::from(file))
        .map_err(|e| format!("Body error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("JSON error: {}", e))
}

/// The message of a hub error response (`{"error": {"code", "message"}}`),
/// or the bare HTTP status if the body isn't one.
async fn error_message(resp: gloo_net::http::Response) -> String {
    let status = resp.status();
    match resp.json::<common::protocol::ErrorBody>().await {
        Ok(body) => body.error.message,
        Err(_) => format!("HTTP {}", status),
    }
}

/// Replace `into` with the hub's active silences. The hub is the source of
/// truth: it assigns the IDs (needed to unmute) and the expiry times.
pub(crate) async fn refresh_silences(into: leptos::prelude::RwSignal<Vec<AlertSilence>>) {
    use leptos::prelude::Set;
    match fetch_json::<Vec<AlertSilence>>("/api/silences").await {
        Ok(list) => into.set(list),
        // Logged out: the layout is already redirecting to /login.
        Err(e) if e == UNAUTHORIZED => {}
        Err(e) => leptos::logging::error!("Reloading silences failed: {e}"),
    }
}
