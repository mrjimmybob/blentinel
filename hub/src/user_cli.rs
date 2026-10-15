#![cfg(feature = "ssr")]
//! `hub user …`: account recovery from the server console (proposal 0001
//! §4.4). The console is trusted (it can read the database anyway), so these
//! run with no acting user; the rules in `service::users` still apply.

use chrono::{DateTime, Utc};

use crate::args::UserCommand;
use crate::model::TemporaryPassword;
use crate::service::audit::{self, Actor, Event, action};
use crate::service::users::{self, UserError, UserFields};
use crate::storage::Storage;

/// Run `command`; returns what to print.
pub async fn run(
    store: &dyn Storage,
    command: &UserCommand,
    now: DateTime<Utc>,
) -> Result<String, UserError> {
    match command {
        UserCommand::List => {
            let users = store.users().await?;
            if users.is_empty() {
                return Ok("No users yet: open the web UI and use the setup code.\n".to_string());
            }
            let mut out = format!(
                "{:<24} {:<11} {:<8} {}\n",
                "USERNAME", "ROLE", "ACTIVE", "LAST LOGIN"
            );
            for u in users {
                out.push_str(&format!(
                    "{:<24} {:<11} {:<8} {}\n",
                    u.username,
                    u.role.as_str(),
                    if u.is_active { "yes" } else { "no" },
                    u.last_login_at.as_deref().unwrap_or("never"),
                ));
            }
            Ok(out)
        }
        UserCommand::Create {
            username,
            role,
            display_name,
        } => {
            if *role == crate::model::Role::Client {
                return Err(UserError::InvalidScope(
                    "create Clients in the web UI (Users), where you choose their companies",
                ));
            }
            let fields = UserFields {
                display_name: display_name.clone().unwrap_or_default(),
                email: None,
                role: *role,
                scope: crate::model::CompanyScope::All,
            };
            let created = users::create_user(store, None, username, fields, None, now).await?;
            let event = Event::new(action::USER_CREATE)
                .target("user", &created.user.id)
                .details(serde_json::json!({ "username": created.user.username, "role": created.user.role, "via": "hub user" }));
            audit::record(store, &Actor::hub(), event, now).await;
            Ok(temporary_password_message("Created", &created))
        }
        UserCommand::ResetPassword { username } => {
            let user = store
                .user_by_username(username)
                .await?
                .ok_or(UserError::NotFound)?;
            let reset = users::reset_password(store, None, &user.info.id).await?;
            let event = Event::new(action::USER_RESET_PASSWORD)
                .target("user", &reset.user.id)
                .details(serde_json::json!({ "username": reset.user.username, "via": "hub user" }));
            audit::record(store, &Actor::hub(), event, now).await;
            Ok(temporary_password_message("Reset the password of", &reset))
        }
    }
}

fn temporary_password_message(what: &str, result: &TemporaryPassword) -> String {
    format!(
        "{what} {} ({}).\nTemporary password: {}\nIt is shown only now; it must be changed at the next login.\n",
        result.user.username,
        result.user.role.label(),
        result.temporary_password
    )
}
