#![cfg(feature = "ssr")]
//! The web UI's files, embedded in the binary by `build.rs` (M2-07, D-11,
//! D-51): a deployed hub is one file, with no `pkg/` folder beside it.
//!
//! A hub built without them (`cargo leptos watch`, tests) has none, and
//! Leptos serves the files from its `site-root` on disk instead.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

/// One embedded file.
#[derive(Debug)]
pub struct WebAsset {
    /// Relative to the site root, with `/`: `pkg/hub.wasm`, `favicon.ico`.
    pub path: &'static str,
    /// Quoted content fingerprint, for `If-None-Match`.
    pub etag: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

/// Whether this binary carries the web UI.
pub fn embedded() -> bool {
    !FILES.is_empty()
}

/// The embedded file for a request path (`/pkg/hub.wasm`), if any.
pub fn find<'a>(files: &'a [WebAsset], request_path: &str) -> Option<&'a WebAsset> {
    let path = request_path.strip_prefix('/')?;
    files.iter().find(|f| f.path == path)
}

/// The answer for `file`: `304` if the browser has this version, else the file.
pub fn respond(file: &WebAsset, request_headers: &HeaderMap) -> Response {
    let etag = HeaderValue::from_static(file.etag);
    // File names aren't content-hashed, so browsers must revalidate; the ETag
    // makes that a 304 unless the hub was upgraded.
    let cache = (header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if request_headers.get(header::IF_NONE_MATCH) == Some(&etag) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag), cache]).into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(file.path)),
            ),
            (header::ETAG, etag),
            cache,
        ],
        Body::from(file.bytes),
    )
        .into_response()
}

/// This binary's file for a request path, answered; `None` if it isn't one.
pub fn serve(request_path: &str, request_headers: &HeaderMap) -> Option<Response> {
    find(FILES, request_path).map(|file| respond(file, request_headers))
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("wasm") => "application/wasm",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &[WebAsset] = &[
        WebAsset {
            path: "pkg/hub.wasm",
            etag: "\"0123456789abcdef\"",
            bytes: b"\0asm",
        },
        WebAsset {
            path: "favicon.ico",
            etag: "\"fedcba9876543210\"",
            bytes: b"ico",
        },
    ];

    #[test]
    fn a_request_path_finds_its_file_and_nothing_else() {
        assert_eq!(find(FAKE, "/pkg/hub.wasm").unwrap().bytes, b"\0asm");
        assert!(find(FAKE, "/favicon.ico").is_some());
        for other in ["/", "/pkg", "/pkg/other.js", "pkg/hub.wasm", "/api/setup"] {
            assert!(find(FAKE, other).is_none(), "{other}");
        }
    }

    #[test]
    fn a_file_is_served_with_its_type_and_etag() {
        let response = respond(&FAKE[0], &HeaderMap::new());
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/wasm");
        assert_eq!(response.headers()[header::ETAG], "\"0123456789abcdef\"");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[test]
    fn a_browser_with_this_version_gets_304() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"0123456789abcdef\""),
        );
        assert_eq!(
            respond(&FAKE[0], &headers).status(),
            StatusCode::NOT_MODIFIED
        );
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"old\""));
        assert_eq!(respond(&FAKE[0], &headers).status(), StatusCode::OK);
    }

    #[test]
    fn types_follow_the_extension() {
        assert_eq!(content_type("pkg/hub.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("pkg/hub.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("favicon.ico"), "image/x-icon");
        assert_eq!(content_type("README"), "application/octet-stream");
    }

    #[test]
    fn a_test_build_embeds_nothing() {
        // Only a build with BLENTINEL_EMBED_SITE embeds the UI.
        if std::env::var("BLENTINEL_EMBED_SITE").is_err() {
            assert!(!embedded());
        }
    }
}
