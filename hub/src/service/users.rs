//! Users, passwords and login sessions (proposal 0001, D-42).
//!
//! The rules live here and nowhere else: password policy, who may manage
//! whom, the last-Owner guard, and when sessions end. Handlers map
//! [`UserError`] to HTTP; the `hub user` CLI calls the same functions with no
//! acting user (the server console is trusted, as for `hub --init`).

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

use crate::model::{Role, TemporaryPassword, UserInfo};
use crate::storage::{NewUser, Storage, StoredUser, UserUpdate};

/// Minimum password length (proposal 0001 §4.3).
pub const MIN_PASSWORD_LEN: usize = 12;
/// A session unused for this long ends.
pub const IDLE_TIMEOUT: Duration = Duration::hours(12);
/// A session ends this long after login, however active.
pub const ABSOLUTE_TIMEOUT: Duration = Duration::days(7);

/// Why a user operation was refused. `Storage` is the only internal error.
#[derive(Debug, thiserror::Error)]
pub enum UserError {
    /// Unknown user, wrong password or deactivated account: deliberately one
    /// answer, so a login attempt can't tell which.
    #[error("wrong username or password")]
    InvalidCredentials,
    #[error("the current password is wrong")]
    WrongCurrentPassword,
    #[error("passwords must be at least {MIN_PASSWORD_LEN} characters")]
    WeakPassword,
    #[error("usernames are 1 to 64 characters: letters, digits, and . _ - @")]
    InvalidUsername,
    #[error("the username {0:?} is already taken")]
    UsernameTaken(String),
    #[error("no such user")]
    NotFound,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("at least one active Owner must remain")]
    LastOwner,
    /// Company scoping is enforced from M1-04; until then only users who see
    /// every company can exist, so no one gets access that only looks limited.
    #[error(
        "users limited to some companies (and the Client role) are not available yet (ROADMAP M1-04)"
    )]
    ScopeNotSupported,
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

pub type UserResult<T> = Result<T, UserError>;

// ---------------------------------------------------------------------------
// Passwords
// ---------------------------------------------------------------------------

fn check_password_rule(password: &str) -> UserResult<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(UserError::WeakPassword);
    }
    Ok(())
}

/// argon2id with the crate's default parameters, off the async threads
/// (hashing is deliberately slow).
pub async fn hash_password(password: &str) -> anyhow::Result<String> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))
    })
    .await?
}

/// Whether `password` matches the PHC string `hash`. A malformed hash never matches.
pub async fn verify_password(hash: &str, password: &str) -> bool {
    let (hash, password) = (hash.to_owned(), password.to_owned());
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// A hash to verify against when the username doesn't exist, so an unknown
/// user takes as long to reject as a wrong password.
async fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    if let Some(hash) = DUMMY.get() {
        return hash;
    }
    let hash = hash_password(&random_string(24)).await.unwrap_or_default();
    DUMMY.get_or_init(|| hash)
}

/// Random characters that are easy to read and type (no 0/O, 1/l/I).
pub fn random_string(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = OsRng;
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// A temporary password: long enough for the rule, shown once.
fn temporary_password() -> String {
    random_string(16)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// SHA-256 (hex) of a session cookie value: what the database stores.
pub fn session_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Start a session for `user_id`; returns the cookie value (256 random bits).
async fn start_session(
    store: &dyn Storage,
    user_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    OsRng.fill(&mut bytes);
    let token = hex::encode(bytes);
    store
        .create_session(&session_hash(&token), user_id, now, now + ABSOLUTE_TIMEOUT)
        .await?;
    Ok(token)
}

/// The user of a live session cookie, refreshing its idle timer.
pub async fn resume(
    store: &dyn Storage,
    token: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<UserInfo>> {
    Ok(store
        .resume_session(&session_hash(token), now, IDLE_TIMEOUT)
        .await?
        .map(|u| u.info))
}

/// Log in: on success, the new session's cookie value and the user.
pub async fn login(
    store: &dyn Storage,
    username: &str,
    password: &str,
    now: DateTime<Utc>,
) -> UserResult<(String, UserInfo)> {
    let user = store.user_by_username(username.trim()).await?;
    let hash = match &user {
        Some(u) => u.password_hash.as_str(),
        None => dummy_hash().await,
    };
    let matches = verify_password(hash, password).await;
    let Some(user) = user.filter(|u| matches && u.info.is_active) else {
        tracing::warn!(username = %username.trim(), "Failed login");
        return Err(UserError::InvalidCredentials);
    };
    store.record_login(&user.info.id, now).await?;
    let token = start_session(store, &user.info.id, now).await?;
    let info = store
        .user(&user.info.id)
        .await?
        .ok_or(UserError::NotFound)?
        .info;
    Ok((token, info))
}

pub async fn logout(store: &dyn Storage, token: &str) -> anyhow::Result<()> {
    store.delete_session(&session_hash(token)).await
}

/// Remove sessions that can't be resumed any more (run periodically).
pub async fn delete_dead_sessions(store: &dyn Storage, now: DateTime<Utc>) -> anyhow::Result<u64> {
    store.delete_dead_sessions(now, IDLE_TIMEOUT).await
}

// ---------------------------------------------------------------------------
// Own account
// ---------------------------------------------------------------------------

/// Change your own password. Your other sessions end; `current_token` stays.
pub async fn change_own_password(
    store: &dyn Storage,
    user_id: &str,
    current_token: &str,
    current_password: &str,
    new_password: &str,
) -> UserResult<()> {
    let user = store.user(user_id).await?.ok_or(UserError::NotFound)?;
    if !verify_password(&user.password_hash, current_password).await {
        return Err(UserError::WrongCurrentPassword);
    }
    check_password_rule(new_password)?;
    store
        .set_password(user_id, &hash_password(new_password).await?, false)
        .await?;
    let keep = session_hash(current_token);
    store.delete_user_sessions(user_id, Some(&keep)).await?;
    Ok(())
}

/// End all your sessions except the current one.
pub async fn logout_other_sessions(
    store: &dyn Storage,
    user_id: &str,
    current_token: &str,
) -> anyhow::Result<()> {
    let keep = session_hash(current_token);
    store.delete_user_sessions(user_id, Some(&keep)).await
}

// ---------------------------------------------------------------------------
// Managing users
// ---------------------------------------------------------------------------

/// The fields of a user being created or edited.
#[derive(Debug, Clone)]
pub struct UserFields {
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    pub all_companies: bool,
    pub companies: Vec<String>,
}

fn check_username(username: &str) -> UserResult<()> {
    let valid_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@');
    if username.is_empty() || username.len() > 64 || !username.chars().all(valid_char) {
        return Err(UserError::InvalidUsername);
    }
    Ok(())
}

/// Normalise the fields and apply the scope rule (M1-04, see [`UserError::ScopeNotSupported`]).
fn checked_fields(mut fields: UserFields, username: &str) -> UserResult<UserFields> {
    fields.display_name = fields.display_name.trim().to_string();
    if fields.display_name.is_empty() {
        fields.display_name = username.to_string();
    }
    fields.email = fields
        .email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    if fields.role == Role::Client || !(fields.role.is_global() || fields.all_companies) {
        return Err(UserError::ScopeNotSupported);
    }
    fields.all_companies = true;
    fields.companies.clear();
    Ok(fields)
}

/// Only an Owner (or the server console, `actor = None`) may create, edit or
/// grant the Owner role.
fn check_may_manage(actor: Option<&UserInfo>, target_role: Role, new_role: Role) -> UserResult<()> {
    let actor_is_owner = actor.is_none_or(|a| a.role == Role::Owner);
    if !actor_is_owner && (target_role == Role::Owner || new_role == Role::Owner) {
        return Err(UserError::Forbidden("only an Owner can manage Owners"));
    }
    Ok(())
}

fn new_user_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill(&mut bytes);
    hex::encode(bytes)
}

/// Create a user. With `password: None`, a temporary password is generated,
/// returned once, and must be changed at first login.
pub async fn create_user(
    store: &dyn Storage,
    actor: Option<&UserInfo>,
    username: &str,
    fields: UserFields,
    password: Option<&str>,
    now: DateTime<Utc>,
) -> UserResult<TemporaryPassword> {
    let username = username.trim();
    check_username(username)?;
    check_may_manage(actor, fields.role, fields.role)?;
    let fields = checked_fields(fields, username)?;
    let (password, must_change) = match password {
        Some(p) => (p.to_string(), false),
        None => (temporary_password(), true),
    };
    check_password_rule(&password)?;
    if store.user_by_username(username).await?.is_some() {
        return Err(UserError::UsernameTaken(username.to_string()));
    }
    let id = new_user_id();
    store
        .create_user(
            &NewUser {
                id: id.clone(),
                username: username.to_string(),
                display_name: fields.display_name,
                email: fields.email,
                password_hash: hash_password(&password).await?,
                role: fields.role,
                all_companies: fields.all_companies,
                companies: fields.companies,
                must_change_password: must_change,
            },
            now,
        )
        .await?;
    let user = store.user(&id).await?.ok_or(UserError::NotFound)?.info;
    Ok(TemporaryPassword {
        user,
        temporary_password: if must_change { password } else { String::new() },
    })
}

async fn target(store: &dyn Storage, id: &str) -> UserResult<StoredUser> {
    store.user(id).await?.ok_or(UserError::NotFound)
}

/// Edit a user. Deactivating ends their sessions.
pub async fn update_user(
    store: &dyn Storage,
    actor: Option<&UserInfo>,
    id: &str,
    fields: UserFields,
    is_active: bool,
) -> UserResult<UserInfo> {
    let current = target(store, id).await?.info;
    check_may_manage(actor, current.role, fields.role)?;
    if !is_active && actor.is_some_and(|a| a.id == id) {
        return Err(UserError::Forbidden("you can't deactivate yourself"));
    }
    let fields = checked_fields(fields, &current.username)?;
    let stays_owner = fields.role == Role::Owner && is_active;
    if current.role == Role::Owner
        && current.is_active
        && !stays_owner
        && store.active_owner_count().await? <= 1
    {
        return Err(UserError::LastOwner);
    }
    store
        .update_user(
            id,
            &UserUpdate {
                display_name: fields.display_name,
                email: fields.email,
                role: fields.role,
                all_companies: fields.all_companies,
                companies: fields.companies,
                is_active,
            },
        )
        .await?;
    if !is_active {
        store.delete_user_sessions(id, None).await?;
    }
    Ok(target(store, id).await?.info)
}

/// Give a user a new temporary password (shown once) and end their sessions.
pub async fn reset_password(
    store: &dyn Storage,
    actor: Option<&UserInfo>,
    id: &str,
) -> UserResult<TemporaryPassword> {
    let current = target(store, id).await?.info;
    check_may_manage(actor, current.role, current.role)?;
    let password = temporary_password();
    store
        .set_password(id, &hash_password(&password).await?, true)
        .await?;
    store.delete_user_sessions(id, None).await?;
    Ok(TemporaryPassword {
        user: target(store, id).await?.info,
        temporary_password: password,
    })
}

/// End all of a user's sessions.
pub async fn force_logout(
    store: &dyn Storage,
    actor: Option<&UserInfo>,
    id: &str,
) -> UserResult<()> {
    let current = target(store, id).await?.info;
    check_may_manage(actor, current.role, current.role)?;
    store.delete_user_sessions(id, None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rule_counts_characters() {
        assert!(check_password_rule("short").is_err());
        assert!(check_password_rule("elevenchars").is_err());
        assert!(check_password_rule("twelve chars").is_ok());
        assert!(
            check_password_rule("ééééééééééé").is_err(),
            "11 chars, 22 bytes"
        );
    }

    #[tokio::test]
    async fn hashes_verify_only_their_password() {
        let hash = hash_password("correct horse battery").await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password(&hash, "correct horse battery").await);
        assert!(!verify_password(&hash, "wrong horse battery").await);
        assert!(!verify_password("not a hash", "anything").await);
    }

    #[test]
    fn usernames_are_restricted() {
        for ok in ["alice", "a.b-c_d", "tech@acme"] {
            assert!(check_username(ok).is_ok(), "{ok}");
        }
        for bad in ["", "has space", "semi;colon", &"x".repeat(65)] {
            assert!(check_username(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn temporary_passwords_satisfy_the_rule() {
        let p = temporary_password();
        assert!(check_password_rule(&p).is_ok());
        assert_ne!(p, temporary_password());
    }

    #[test]
    fn session_hash_is_sha256_hex() {
        assert_eq!(session_hash("abc").len(), 64);
        assert_ne!(session_hash("abc"), session_hash("abd"));
    }
}
