//! Managing alert channels, routing rules and the hub's alert settings (the
//! alerts page, D-66). Secrets are sealed on the way in and never come back
//! out: an answer says only whether one is set.

use chrono::{DateTime, Utc};

use super::{Delivery, Notification};
use crate::model::{
    AlertSettings, AlertsOverview, ChannelInfo, ChannelRequest, CompanyScope, RouteTarget,
    RoutesRequest, TestResult,
};
use crate::storage::{Storage, StoredAlertSettings, StoredChannel};

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("no such channel")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

fn format_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn info(channel: &StoredChannel) -> ChannelInfo {
    ChannelInfo {
        id: channel.id,
        name: channel.name.clone(),
        kind: channel.kind,
        enabled: channel.enabled,
        target: channel.target.clone(),
        secret_set: channel.secret_sealed.is_some(),
        last_delivery_at: channel.last_delivery_at.map(format_time),
        last_error: channel.last_error.clone(),
    }
}

fn settings_view(stored: Option<StoredAlertSettings>) -> AlertSettings {
    match stored {
        None => AlertSettings {
            smtp_port: 587,
            ..AlertSettings::default()
        },
        Some(s) => AlertSettings {
            hub_url: s.hub_url,
            smtp_server: s.smtp_server,
            smtp_port: s.smtp_port,
            smtp_security: s.smtp_security,
            smtp_username: s.smtp_username,
            smtp_from: s.smtp_from,
            smtp_password: None,
            smtp_password_set: s.smtp_password_sealed.is_some(),
        },
    }
}

/// Everything the alerts page shows.
pub async fn overview(store: &dyn Storage) -> anyhow::Result<AlertsOverview> {
    let mut companies = Vec::new();
    for company in store.companies(&CompanyScope::All).await? {
        let sites = store
            .company(&company.id)
            .await?
            .map(|d| d.sites.into_iter().map(|s| (s.id, s.name)).collect())
            .unwrap_or_default();
        companies.push(RouteTarget {
            company_id: company.id,
            company_name: company.name,
            sites,
        });
    }
    Ok(AlertsOverview {
        settings: settings_view(store.alert_settings().await?),
        channels: store.channels().await?.iter().map(info).collect(),
        rules: store.route_rules().await?,
        companies,
    })
}

/// Save the hub's alert settings. The password: `None` keeps it, empty clears it.
pub async fn save_settings(
    store: &dyn Storage,
    delivery: &Delivery,
    settings: &AlertSettings,
    by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), ChannelError> {
    settings.validate().map_err(ChannelError::Invalid)?;
    let current = store.alert_settings().await?;
    let password_sealed = match settings.smtp_password.as_deref() {
        None => current.and_then(|c| c.smtp_password_sealed),
        Some("") => None,
        Some(password) => Some(delivery.key.seal(password)?),
    };
    let stored = StoredAlertSettings {
        hub_url: settings.hub_url.trim().trim_end_matches('/').to_string(),
        smtp_server: settings.smtp_server.trim().to_string(),
        smtp_port: settings.smtp_port,
        smtp_security: settings.smtp_security,
        smtp_username: settings.smtp_username.trim().to_string(),
        smtp_from: settings.smtp_from.trim().to_string(),
        smtp_password_sealed: password_sealed,
    };
    store.set_alert_settings(&stored, by, now).await?;
    Ok(())
}

/// The secret a request gives, if any (empty means "keep").
fn given_secret(request: &ChannelRequest) -> Option<&str> {
    request
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Add a channel.
pub async fn create(
    store: &dyn Storage,
    delivery: &Delivery,
    request: &ChannelRequest,
    now: DateTime<Utc>,
) -> Result<ChannelInfo, ChannelError> {
    request.validate(false).map_err(ChannelError::Invalid)?;
    let channel = StoredChannel {
        id: 0,
        name: request.name.trim().to_string(),
        kind: request.kind,
        enabled: request.enabled,
        target: request.target.trim().to_string(),
        secret_sealed: given_secret(request)
            .map(|s| delivery.key.seal(s))
            .transpose()?,
        last_delivery_at: None,
        last_error: None,
    };
    let id = store.create_channel(&channel, now).await?;
    let saved = store.channel(id).await?.ok_or(ChannelError::NotFound)?;
    Ok(info(&saved))
}

/// Change a channel. Its secret stays unless a new one is given (a new
/// kind needs a new one: the old one meant something else).
pub async fn update(
    store: &dyn Storage,
    delivery: &Delivery,
    id: i64,
    request: &ChannelRequest,
) -> Result<ChannelInfo, ChannelError> {
    let current = store.channel(id).await?.ok_or(ChannelError::NotFound)?;
    let kept = (current.kind == request.kind)
        .then_some(current.secret_sealed)
        .flatten();
    request
        .validate(kept.is_some())
        .map_err(ChannelError::Invalid)?;
    let secret_sealed = match given_secret(request) {
        Some(secret) => Some(delivery.key.seal(secret)?),
        None => kept,
    };
    let channel = StoredChannel {
        name: request.name.trim().to_string(),
        kind: request.kind,
        enabled: request.enabled,
        target: request.target.trim().to_string(),
        secret_sealed,
        ..current
    };
    if !store.update_channel(&channel).await? {
        return Err(ChannelError::NotFound);
    }
    let saved = store.channel(id).await?.ok_or(ChannelError::NotFound)?;
    Ok(info(&saved))
}

/// Remove a channel, and from the rules that named it (a rule left with no
/// channel goes too). Returns its name.
pub async fn delete(store: &dyn Storage, id: i64) -> Result<String, ChannelError> {
    let channel = store.channel(id).await?.ok_or(ChannelError::NotFound)?;
    let mut rules = store.route_rules().await?;
    for rule in &mut rules {
        rule.channels.retain(|c| *c != id);
        rule.escalate_to.retain(|c| *c != id);
        if rule.escalate_to.is_empty() {
            rule.escalate_after_minutes = None;
        }
    }
    rules.retain(|r| !r.channels.is_empty());
    store.set_route_rules(&rules).await?;
    store.delete_channel(id).await?;
    Ok(channel.name)
}

/// Replace the routing rules.
pub async fn save_rules(store: &dyn Storage, request: &RoutesRequest) -> Result<(), ChannelError> {
    let ids: Vec<i64> = store.channels().await?.iter().map(|c| c.id).collect();
    request.validate(&ids).map_err(ChannelError::Invalid)?;
    store.set_route_rules(&request.rules).await?;
    Ok(())
}

/// "Send test": a made-up alert to one channel, once, now.
pub async fn test(
    store: &dyn Storage,
    delivery: &Delivery,
    id: i64,
) -> Result<TestResult, ChannelError> {
    let stored = store.channel(id).await?.ok_or(ChannelError::NotFound)?;
    let result = async {
        let channel = delivery.open(&stored)?;
        let sending = delivery.sending(store).await?;
        let sample = Notification::sample(delivery.clock.now());
        delivery.try_once(store, &channel, &sample, &sending).await
    }
    .await;
    Ok(match result {
        Ok(()) => TestResult {
            ok: true,
            error: None,
        },
        Err(e) => TestResult {
            ok: false,
            error: Some(format!("{e:#}")),
        },
    })
}
