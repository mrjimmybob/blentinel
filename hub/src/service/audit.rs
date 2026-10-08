//! The audit log: who did what, when, from where (M1-06, D-46).
//!
//! Handlers record an [`Event`] after an action succeeds (and for failed
//! logins). Recording is best effort: a failure is logged but never undoes or
//! blocks the action, so a full disk can't lock everyone out. The action
//! names are the `action` constants below; `docs/guide/audit.md` lists them.

use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{AuditEntry, AuditQuery, UserInfo};
use crate::storage::{AuditFilter, NewAuditEntry, Storage};

/// The action names, grouped by prefix (the filter matches a prefix).
pub mod action {
    pub const LOGIN: &str = "login.success";
    pub const LOGIN_FAILED: &str = "login.failed";
    pub const LOGOUT: &str = "login.logout";
    pub const SETUP: &str = "setup.complete";
    pub const SETUP_FAILED: &str = "setup.failed";
    pub const PASSWORD_CHANGE: &str = "account.password_change";
    pub const LOGOUT_OTHERS: &str = "account.logout_others";
    pub const USER_CREATE: &str = "user.create";
    pub const USER_UPDATE: &str = "user.update";
    pub const USER_RESET_PASSWORD: &str = "user.reset_password";
    pub const USER_LOGOUT: &str = "user.force_logout";
    pub const COMPANY_CREATE: &str = "company.create";
    pub const COMPANY_UPDATE: &str = "company.update";
    pub const COMPANY_ARCHIVE: &str = "company.archive";
    pub const COMPANY_UNARCHIVE: &str = "company.unarchive";
    pub const SITE_CREATE: &str = "site.create";
    pub const SITE_UPDATE: &str = "site.update";
    pub const THRESHOLDS_UPDATE: &str = "thresholds.update";
    pub const SILENCE_CREATE: &str = "silence.create";
    pub const SILENCE_DELETE: &str = "silence.delete";
    pub const MAINTENANCE_CREATE: &str = "maintenance.create";
    pub const MAINTENANCE_DELETE: &str = "maintenance.delete";
    pub const INCIDENT_ACKNOWLEDGE: &str = "incident.acknowledge";
    pub const INCIDENT_NOTE: &str = "incident.note";
    pub const ALERTS_SETTINGS: &str = "alerts.settings";
    pub const CHANNEL_CREATE: &str = "channel.create";
    pub const CHANNEL_UPDATE: &str = "channel.update";
    pub const CHANNEL_DELETE: &str = "channel.delete";
    pub const ROUTES_UPDATE: &str = "alerts.routes";
    pub const DIGEST_SETTINGS: &str = "alerts.digest";
    pub const DATA_DELETE_COMPANY: &str = "data.delete_company";
    pub const DATA_DELETE_PROBE: &str = "data.delete_probe";
    pub const PROBE_REMOVE: &str = "probe.remove";
    pub const PROBE_ENROLL: &str = "probe.enroll";
    pub const PROBE_UPDATE: &str = "probe.update";
    pub const PROBE_REVOKE: &str = "probe.revoke";
    pub const CODE_CREATE: &str = "code.create";
    pub const CODE_REVOKE: &str = "code.revoke";
    pub const CHECK_CREATE: &str = "check.create";
    pub const CHECK_UPDATE: &str = "check.update";
    pub const CHECK_DELETE: &str = "check.delete";
    pub const CHECK_COPY: &str = "check.copy";
    pub const COMMAND_QUEUE: &str = "command.queue";
    pub const COMMAND_CANCEL: &str = "command.cancel";
    pub const ARCHIVE_CREATE: &str = "archive.create";
    pub const BACKUP_CREATE: &str = "backup.create";
    pub const BACKUP_DELETE: &str = "backup.delete";
    pub const BACKUP_DOWNLOAD: &str = "backup.download";
    pub const BACKUP_DOWNLOAD_FAILED: &str = "backup.download_failed";
    pub const BACKUP_SCHEDULE: &str = "backup.schedule";
    pub const ROTATION_START: &str = "rotation.start";
    pub const ROTATION_SWITCH: &str = "rotation.switch";
    pub const ROTATION_CANCEL: &str = "rotation.cancel";
    pub const CERTIFICATE_SETTINGS: &str = "certificate.settings";
    pub const CERTIFICATE_ISSUE: &str = "certificate.issue";
    pub const CERTIFICATE_ISSUE_FAILED: &str = "certificate.issue_failed";
    pub const RELEASE_ADD: &str = "release.add";
    pub const RELEASE_REMOVE: &str = "release.remove";
    pub const RELEASE_APPROVE: &str = "release.approve";
}

/// Who acted.
#[derive(Debug, Clone, Default)]
pub struct Actor {
    user_id: Option<String>,
    username: Option<String>,
    ip: Option<String>,
}

impl Actor {
    /// A logged-in user.
    pub fn user(user: &UserInfo, ip: Option<IpAddr>) -> Self {
        Self {
            user_id: Some(user.id.clone()),
            username: Some(user.username.clone()),
            ip: ip.map(|ip| ip.to_string()),
        }
    }

    /// Someone not (yet) logged in, e.g. a failed login under `username`.
    pub fn anonymous(username: Option<&str>, ip: Option<IpAddr>) -> Self {
        Self {
            user_id: None,
            username: username.map(|u| u.trim().to_string()),
            ip: ip.map(|ip| ip.to_string()),
        }
    }

    /// The hub itself (e.g. a company registered by a report) or the server
    /// console (`hub user …`).
    pub fn hub() -> Self {
        Self::default()
    }
}

/// What happened.
#[derive(Debug, Clone)]
pub struct Event {
    action: &'static str,
    target_type: Option<&'static str>,
    target_id: Option<String>,
    company_id: Option<String>,
    details: Option<Value>,
}

impl Event {
    pub fn new(action: &'static str) -> Self {
        Self {
            action,
            target_type: None,
            target_id: None,
            company_id: None,
            details: None,
        }
    }

    pub fn target(mut self, kind: &'static str, id: impl Into<String>) -> Self {
        self.target_type = Some(kind);
        self.target_id = Some(id.into());
        self
    }

    pub fn company(mut self, company_id: impl Into<String>) -> Self {
        self.company_id = Some(company_id.into());
        self
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Record `event` by `actor`. Never fails the caller (see the module docs).
pub async fn record(store: &dyn Storage, actor: &Actor, event: Event, now: DateTime<Utc>) {
    let entry = NewAuditEntry {
        at: now,
        user_id: actor.user_id.clone(),
        username: actor.username.clone(),
        actor_ip: actor.ip.clone(),
        action: event.action.to_string(),
        target_type: event.target_type.map(str::to_string),
        target_id: event.target_id,
        company_id: event.company_id,
        details: event.details.map(|d| d.to_string()),
    };
    if let Err(e) = store.append_audit(&entry).await {
        tracing::error!(
            action = event.action,
            "Could not write the audit log: {e:#}"
        );
    }
}

/// The largest page `GET /api/audit` returns.
pub const MAX_LIMIT: u32 = 1000;
/// The page size without `limit`.
pub const DEFAULT_LIMIT: u32 = 200;

/// The API's query as a storage filter; `Err` names the bad parameter.
pub fn filter(query: &AuditQuery) -> Result<AuditFilter, String> {
    let time = |name: &str, value: &Option<String>| -> Result<Option<DateTime<Utc>>, String> {
        value
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .map(|v| {
                DateTime::parse_from_rfc3339(v.trim())
                    .map(|t| t.with_timezone(&Utc))
                    .map_err(|_| {
                        format!("{name} must be an RFC 3339 time, e.g. 2026-09-28T00:00:00Z")
                    })
            })
            .transpose()
    };
    let text = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Ok(AuditFilter {
        username: text(&query.user),
        action_prefix: text(&query.action),
        target: text(&query.target),
        from: time("from", &query.from)?,
        to: time("to", &query.to)?,
        before_id: query.before_id,
        limit: query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
    })
}

/// Entries as CSV (RFC 4180), with a header row.
pub fn to_csv(entries: &[AuditEntry]) -> String {
    fn field(value: Option<&str>) -> String {
        let value = value.unwrap_or("");
        if value.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_string()
        }
    }
    let mut csv = String::from("id,at,user,ip,action,target_type,target_id,company,details\r\n");
    for e in entries {
        let id = e.id.to_string();
        let row = [
            Some(id.as_str()),
            Some(e.at.as_str()),
            e.username.as_deref(),
            e.actor_ip.as_deref(),
            Some(e.action.as_str()),
            e.target_type.as_deref(),
            e.target_id.as_deref(),
            e.company_id.as_deref(),
            e.details.as_deref(),
        ]
        .map(field)
        .join(",");
        csv.push_str(&row);
        csv.push_str("\r\n");
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parameters_become_a_filter() {
        let query = AuditQuery {
            user: Some(" alice ".into()),
            action: Some("".into()),
            from: Some("2026-09-28T00:00:00Z".into()),
            limit: Some(50_000),
            ..Default::default()
        };
        let f = filter(&query).unwrap();
        assert_eq!(f.username.as_deref(), Some("alice"));
        assert_eq!(f.action_prefix, None, "empty means no filter");
        assert!(f.from.is_some() && f.to.is_none());
        assert_eq!(f.limit, MAX_LIMIT);
        assert_eq!(filter(&AuditQuery::default()).unwrap().limit, DEFAULT_LIMIT);

        let bad = AuditQuery {
            to: Some("yesterday".into()),
            ..Default::default()
        };
        assert!(filter(&bad).unwrap_err().starts_with("to must be"));
    }

    #[test]
    fn csv_quotes_what_needs_quoting() {
        let entry = AuditEntry {
            id: 7,
            at: "2026-09-28T10:00:00Z".into(),
            username: Some("alice".into()),
            actor_ip: None,
            action: "company.update".into(),
            target_type: Some("company".into()),
            target_id: Some("acme".into()),
            company_id: Some("acme".into()),
            details: Some(r#"{"name":"ACME, Ltd"}"#.into()),
        };
        let csv = to_csv(&[entry]);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("id,at,user,ip,action,target_type,target_id,company,details")
        );
        assert_eq!(
            lines.next(),
            Some(
                r#"7,2026-09-28T10:00:00Z,alice,,company.update,company,acme,acme,"{""name"":""ACME, Ltd""}""#
            )
        );
    }
}
