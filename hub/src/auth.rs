#![cfg(feature = "ssr")]

use axum::extract::{FromRequestParts, FromRef};
use axum::http::request::Parts;
use axum::response::{Response, IntoResponse};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{Utc, DateTime};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// session_id (32-char hex) → expiry timestamp
pub type SessionStore = Arc<Mutex<HashMap<String, DateTime<Utc>>>>;

pub const SESSION_COOKIE_NAME: &str = "blentinel_session";

// ---------------------------------------------------------------------------
// Session store constructor
// ---------------------------------------------------------------------------

pub fn new_session_store() -> SessionStore {
    Arc::new(Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Token lifecycle
// ---------------------------------------------------------------------------

/// Load the admin token from disk, or generate one on first run.
pub fn load_or_create_token(path: &str) -> String {
    use std::path::Path;
    use std::fs;

    if Path::new(path).exists() {
        let token = fs::read_to_string(path)
            .expect("Failed to read auth token file");
        let token = token.trim().to_string();
        if token.is_empty() {
            panic!("Auth token file {} exists but is empty", path);
        }
        println!("Auth token loaded from {}", path);
        token
    } else {
        use rand::Rng;
        let mut rng = rand::rngs::OsRng;
        let bytes: [u8; 16] = {
            let mut arr = [0u8; 16];
            rng.fill(&mut arr);
            arr
        };
        let token = hex::encode(&bytes);

        std::fs::write(path, &token)
            .expect("Failed to write auth token file");

        println!("\n================================================");
        println!("FIRST RUN: New Admin Token Generated");
        println!("================================================");
        println!("ADMIN TOKEN: {}", token);
        println!("\nCopy this token and enter it in the web UI to log in.");
        println!("Token saved to: {}", path);
        println!("================================================\n");

        token
    }
}

// ---------------------------------------------------------------------------
// Session operations
// ---------------------------------------------------------------------------

/// Create a new session, returning the session ID.
pub fn create_session(store: &SessionStore) -> String {
    use rand::Rng;
    let mut rng = rand::rngs::OsRng;
    let bytes: [u8; 16] = {
        let mut arr = [0u8; 16];
        rng.fill(&mut arr);
        arr
    };
    let session_id = hex::encode(&bytes);
    let expiry = Utc::now() + chrono::Duration::hours(24);

    store.lock().unwrap().insert(session_id.clone(), expiry);
    session_id
}

/// Validate a session ID. Lazily removes expired sessions.
pub fn validate_session(store: &SessionStore, id: &str) -> bool {
    let mut sessions = store.lock().unwrap();
    match sessions.get(id) {
        Some(&expiry) => {
            if Utc::now() > expiry {
                sessions.remove(id);
                false
            } else {
                true
            }
        }
        None => false,
    }
}

/// Destroy (log out) a session.
pub fn destroy_session(store: &SessionStore, id: &str) {
    store.lock().unwrap().remove(id);
}

// ---------------------------------------------------------------------------
// Cookie parsing
// ---------------------------------------------------------------------------

/// Extract the blentinel_session cookie value from a Cookie header string.
pub fn extract_session_cookie(cookie_header: &str) -> Option<&str> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("blentinel_session=") {
            return Some(value.trim());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Axum extractor
// ---------------------------------------------------------------------------

/// Axum extractor that validates the session cookie.
/// If no valid session is found, returns a 401 response automatically.
pub struct AuthSession {
    pub session_id: String,
}

impl<S> FromRequestParts<S> for AuthSession
where
    S: Send + Sync,
    SessionStore: FromRef<S>,
{
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let store = SessionStore::from_ref(state);

        let cookie_header = parts
            .headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        async move {
            let session_id = extract_session_cookie(&cookie_header)
                .unwrap_or("")
                .to_string();

            if session_id.is_empty() || !validate_session(&store, &session_id) {
                return Err(axum::http::StatusCode::UNAUTHORIZED.into_response());
            }

            Ok(AuthSession { session_id })
        }
    }
}
