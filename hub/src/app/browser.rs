//! The browser APIs the UI uses (client only). A missing `window` or
//! `document` can't happen in a real browser; if it does, these do nothing
//! instead of panicking.

use leptos::prelude::{StoredValue, on_cleanup};
use leptos::wasm_bindgen::{JsCast, closure::Closure};
use leptos::web_sys;

const THEME_KEY: &str = "blentinel_theme";

/// Go to `path` (a full page load).
pub(crate) fn navigate(path: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(path);
    }
}

/// This page's origin, e.g. `https://hub.example.com:3443`.
pub(crate) fn origin() -> String {
    web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default()
}

pub(crate) fn reload() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

/// The theme saved by [`store_theme`], if any.
pub(crate) fn stored_theme() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(THEME_KEY)
        .ok()?
}

pub(crate) fn store_theme(theme: &str) {
    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
        let _ = storage.set_item(THEME_KEY, theme);
    }
}

/// Switch the page's theme (`data-theme` on `<html>`, used by the CSS).
pub(crate) fn apply_theme(theme: &str) {
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-theme", theme);
    }
}

/// Call `tick` every `ms` milliseconds until the calling component unmounts.
pub(crate) fn poll_every(ms: i32, tick: impl FnMut() + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::<dyn FnMut()>::new(tick);
    let Ok(interval_id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
        callback.as_ref().unchecked_ref(),
        ms,
    ) else {
        return;
    };
    // The component's owner keeps the closure alive while the interval runs and
    // frees it on unmount, right after the interval is cleared.
    StoredValue::new_local(callback);
    on_cleanup(move || {
        if let Some(window) = web_sys::window() {
            window.clear_interval_with_handle(interval_id);
        }
    });
}
