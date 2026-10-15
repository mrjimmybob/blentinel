//! First run: the setup code that lets the first Owner be created (proposal
//! 0001 §4.4, D-42).
//!
//! A hub with no users generates a one-time code, logs it, and writes it to
//! `<state_dir>/setup.code` (owner-only). Whoever creates the first Owner
//! must present it, so a freshly installed hub reachable from the internet
//! can't be claimed by someone else. The file matters on Windows, where the
//! hub runs as a SYSTEM task whose log is hard to reach.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::model::{Role, UserInfo};
use crate::service::users::{self, UserError, UserFields};
use crate::storage::Storage;

/// File name of the setup code inside `state_dir`.
pub const SETUP_CODE_FILE: &str = "setup.code";

/// Why setup was refused.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("this hub is already set up; log in instead")]
    AlreadyDone,
    #[error("wrong setup code")]
    WrongCode,
    #[error(transparent)]
    User(#[from] UserError),
}

/// The current setup code, if the hub still needs one.
#[derive(Debug, Default)]
pub struct SetupCode {
    /// (code, file it was written to). The lock also serialises setup attempts.
    current: Mutex<Option<(String, PathBuf)>>,
}

impl SetupCode {
    /// If the database has no users and no code is pending, create a code,
    /// log it and write it to `state_dir`. Called at startup and after a
    /// reload switches the database.
    pub async fn ensure(&self, store: &dyn Storage, state_dir: &Path) -> anyhow::Result<()> {
        let mut current = self.current.lock().await;
        if store.user_count().await? > 0 {
            if let Some((_, path)) = current.take() {
                remove_file(&path);
            }
            return Ok(());
        }
        if current.is_some() {
            return Ok(());
        }
        let code = users::random_string(8).to_uppercase();
        let path = state_dir.join(SETUP_CODE_FILE);
        common::fs::write_secret_file(&path, format!("{code}\n"))?;
        tracing::warn!(
            "No users yet. Open the web UI and create the first Owner with setup code {code} \
             (also in {})",
            path.display()
        );
        *current = Some((code, path));
        Ok(())
    }

    /// Create the first Owner, if `code` is right and no user exists yet.
    pub async fn complete(
        &self,
        store: &dyn Storage,
        code: &str,
        username: &str,
        display_name: &str,
        password: &str,
        now: DateTime<Utc>,
    ) -> Result<UserInfo, SetupError> {
        let mut current = self.current.lock().await;
        if store.user_count().await.map_err(UserError::from)? > 0 {
            return Err(SetupError::AlreadyDone);
        }
        let Some((expected, path)) = current.as_ref() else {
            return Err(SetupError::WrongCode);
        };
        if !same_code(expected, code) {
            tracing::warn!("Setup attempted with a wrong code");
            return Err(SetupError::WrongCode);
        }
        let fields = UserFields {
            display_name: display_name.to_string(),
            email: None,
            role: Role::Owner,
            scope: crate::model::CompanyScope::All,
        };
        let created =
            users::create_user(store, None, username, fields, Some(password), now).await?;
        remove_file(path);
        *current = None;
        tracing::info!(username = %created.user.username, "First Owner created; setup complete");
        Ok(created.user)
    }
}

/// Compare codes in constant time (by comparing digests), ignoring case and
/// surrounding spaces.
fn same_code(expected: &str, given: &str) -> bool {
    let digest = |s: &str| Sha256::digest(s.trim().to_uppercase().as_bytes());
    digest(expected) == digest(given)
}

fn remove_file(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("Could not delete {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_compare_ignoring_case_and_spaces() {
        assert!(same_code("AB23CD45", " ab23cd45 "));
        assert!(!same_code("AB23CD45", "AB23CD46"));
        assert!(!same_code("AB23CD45", ""));
    }
}
