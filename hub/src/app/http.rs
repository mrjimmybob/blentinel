//! Browser-side calls to the hub's JSON API (compiled for the client only).

use gloo_net::http::Request;

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

/// POST a plain-text body and return the status (login, logout).
pub(crate) async fn post_text(url: &str, body: &str) -> Result<u16, String> {
    let resp = Request::post(url)
        .header("Content-Type", "text/plain")
        .body(body)
        .map_err(|e| format!("Body error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    Ok(resp.status())
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
