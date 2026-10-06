//! The hub's certificate from Let's Encrypt or another ACME authority (M2-14,
//! proposal 0004 §4.4, D-62).
//!
//! The certificate is always issued **for the hub's existing TLS key** (the
//! request is signed with it), so what probes pin, the SHA-256 of the
//! certificate's public key (D-61), never changes: turning ACME on, every
//! renewal, and turning it off again are invisible to probes. A new key comes
//! only from a rotation (D-61); while one announces a new certificate key,
//! the next certificate is issued too, so the switch serves a trusted one.
//!
//! The authority checks the hub answers for its names with TLS-ALPN-01 (on
//! the HTTPS listener, port 443 from outside) or HTTP-01 (on the plain-HTTP
//! listener, port 80 from outside); the answers wait in
//! [`crate::tls::AcmeChallenges`] for the order's length.

use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use crate::config::HubConfig;
use crate::keys::KeyFiles;
use crate::model::{AcmeChallenge, AcmeSettings, CertificateInfo, CertificateOverview};
use crate::storage::{AcmeStatus, Storage};
use crate::tls::{self, AcmeChallenges, CertificateDetails};

/// How often the hub looks whether a certificate is due.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(3600);

/// The longest an order may take, challenges included.
const ORDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// The first retry after a failure; each further failure doubles it, up to
/// [`MAX_RETRY`]. Failed validations count against the authority's limits
/// (Let's Encrypt: 5 per name per hour), so retries back off.
const FIRST_RETRY: Duration = Duration::hours(1);
const MAX_RETRY: Duration = Duration::hours(24);

/// One order at a time: the hourly look, "Renew now" and a save may race.
static ISSUING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// What is asked of the authority for one certificate.
pub struct Order<'a> {
    pub settings: &'a AcmeSettings,
    /// The request (DER), signed with the hub's TLS key.
    pub csr_der: Vec<u8>,
    /// Where the ACME account's key is kept (made on first use).
    pub account_file: &'a Path,
}

/// An ACME authority. Production: [`AcmeClient`]; tests: their own.
#[async_trait]
pub trait CertificateAuthority: Send + Sync {
    /// Prove the hub answers for the order's names, answering the
    /// authority's challenges through `challenges`, and return the issued
    /// certificate chain (PEM).
    async fn issue(&self, order: &Order<'_>, challenges: &AcmeChallenges)
    -> anyhow::Result<String>;
}

/// A certificate the hub keeps issued: the one it serves, or the next one
/// of a rotation that announces a new certificate key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// The one the listener serves (reload it after issuing).
    pub serving: bool,
}

/// Why the certificate should be issued now; `None`: it's fine as it is.
pub fn renewal_reason(
    settings: &AcmeSettings,
    cert: Option<&CertificateDetails>,
    now: DateTime<Utc>,
) -> Option<String> {
    let Some(cert) = cert else {
        return Some("there is no certificate".into());
    };
    if cert.self_signed {
        return Some("the certificate is self-signed".into());
    }
    let mut names = cert.names.clone();
    names.sort();
    let mut wanted = settings.domains.clone();
    wanted.sort();
    if names != wanted {
        return Some("the names changed".into());
    }
    let renew_at = renew_at(cert);
    (now >= renew_at).then(|| {
        format!(
            "it expires {} (renewed with a third of its life left)",
            cert.not_after.format("%Y-%m-%d")
        )
    })
}

/// When a certificate is renewed: once a third of its life is left, as
/// Let's Encrypt advises (30 days of 90; it works for shorter ones too).
pub fn renew_at(cert: &CertificateDetails) -> DateTime<Utc> {
    cert.not_after - cert.lifetime() / 3
}

/// When the hub may try again after `status`'s failures.
pub fn retry_at(status: &AcmeStatus) -> DateTime<Utc> {
    if status.failures == 0 {
        return status.last_attempt_at;
    }
    let doublings = (status.failures - 1).min(10);
    let wait = (FIRST_RETRY * 2_i32.pow(doublings)).min(MAX_RETRY);
    status.last_attempt_at + wait
}

/// The settings in force (off if never set).
pub async fn settings(store: &dyn Storage) -> anyhow::Result<AcmeSettings> {
    Ok(store.acme_settings().await?.unwrap_or_default())
}

/// The certificates to keep issued: the served one and, while a rotation
/// announces a new certificate, the next one.
pub async fn targets(store: &dyn Storage, config: &HubConfig) -> anyhow::Result<Vec<Target>> {
    let files = KeyFiles::of(config);
    let mut targets = vec![Target {
        cert: files.tls_cert.clone(),
        key: files.tls_key.clone(),
        serving: true,
    }];
    let rotating_tls = super::rotation::active(store)
        .await?
        .is_some_and(|r| r.rotates_tls);
    if rotating_tls && files.tls_key_next.exists() {
        targets.push(Target {
            cert: files.tls_cert_next,
            key: files.tls_key_next,
            serving: false,
        });
    }
    Ok(targets)
}

fn details_of(path: &Path) -> Option<CertificateDetails> {
    let pem = std::fs::read(path).ok()?;
    tls::certificate_details(&pem).ok()
}

/// What happened when the hub looked.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Off, or every certificate is fine.
    Nothing,
    /// The last attempt failed; the next waits until then.
    Waiting(DateTime<Utc>),
    /// Issued: `serving` says whether the listener must load the new one.
    Issued { count: usize, serving: bool },
    /// The attempt failed (recorded; the page shows it).
    Failed(String),
}

/// Look whether a certificate is due and, if so, ask the authority for it.
/// `force` ("Renew now", new settings): ask now, for every target, whatever
/// their state and past failures.
pub async fn run(
    store: &dyn Storage,
    config: &HubConfig,
    authority: &dyn CertificateAuthority,
    challenges: &AcmeChallenges,
    force: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<Outcome> {
    let _one_at_a_time = ISSUING.lock().await;
    let settings = settings(store).await?;
    if !settings.enabled || !config.server.tls.enabled {
        return Ok(Outcome::Nothing);
    }
    if !force
        && let Some(status) = store.acme_status().await?
        && status.failures > 0
        && now < retry_at(&status)
    {
        return Ok(Outcome::Waiting(retry_at(&status)));
    }
    let mut due = Vec::new();
    for target in targets(store, config).await? {
        let reason = match force {
            true => Some("asked now".to_string()),
            false => renewal_reason(&settings, details_of(&target.cert).as_ref(), now),
        };
        if let Some(reason) = reason {
            tracing::info!(certificate = %target.cert.display(), "ACME: requesting a certificate: {reason}");
            due.push(target);
        }
    }
    if due.is_empty() {
        return Ok(Outcome::Nothing);
    }
    let account_file = KeyFiles::of(config).acme_account;
    let mut serving = false;
    for target in &due {
        let issued = issue(&settings, target, &account_file, authority, challenges).await;
        challenges.clear();
        if let Err(e) = issued {
            let error = format!("{e:#}");
            store.record_acme_attempt(now, Some(&error)).await?;
            tracing::error!("ACME: no certificate: {error}");
            return Ok(Outcome::Failed(error));
        }
        serving |= target.serving;
    }
    store.record_acme_attempt(now, None).await?;
    tracing::info!(count = due.len(), "ACME: certificate issued");
    Ok(Outcome::Issued {
        count: due.len(),
        serving,
    })
}

/// Ask for a certificate for `target`'s key and write it in place.
async fn issue(
    settings: &AcmeSettings,
    target: &Target,
    account_file: &Path,
    authority: &dyn CertificateAuthority,
    challenges: &AcmeChallenges,
) -> anyhow::Result<()> {
    let key_pem = std::fs::read(&target.key)
        .with_context(|| format!("can't read {}", target.key.display()))?;
    let order = Order {
        settings,
        csr_der: tls::csr_der(&key_pem, &settings.domains)?,
        account_file,
    };
    let chain = tokio::time::timeout(ORDER_TIMEOUT, authority.issue(&order, challenges))
        .await
        .map_err(|_| anyhow::anyhow!("the authority took longer than {ORDER_TIMEOUT:?}"))??;
    check_issued(chain.as_bytes(), &key_pem, settings)?;
    common::fs::write_atomic(&target.cert, chain.as_bytes())
        .with_context(|| format!("can't write {}", target.cert.display()))?;
    Ok(())
}

/// The certificate is for the hub's key (so probes' pin holds) and its names.
fn check_issued(chain_pem: &[u8], key_pem: &[u8], settings: &AcmeSettings) -> anyhow::Result<()> {
    anyhow::ensure!(
        tls::cert_spki_sha256(chain_pem)? == tls::key_spki_sha256(key_pem)?,
        "the authority's certificate isn't for the hub's key"
    );
    let details = tls::certificate_details(chain_pem)?;
    for domain in &settings.domains {
        anyhow::ensure!(
            details.names.contains(domain),
            "the authority's certificate isn't for {domain}"
        );
    }
    Ok(())
}

/// What changing the settings needs done.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    /// Off before and after, or nothing that matters changed.
    Nothing,
    /// Ask the authority now (switched on, or the names or the authority
    /// changed). A new contact or challenge alone doesn't spend an issuance
    /// (authorities limit them: Let's Encrypt, 5 a week for the same names).
    Issue,
    /// Switched off: go back to a self-signed certificate (same key).
    SelfSigned,
}

/// What going from `old` to `new` settings needs done.
pub fn change(old: &AcmeSettings, new: &AcmeSettings) -> Change {
    match (old.enabled, new.enabled) {
        (_, true)
            if !old.enabled || old.domains != new.domains || old.directory() != new.directory() =>
        {
            Change::Issue
        }
        (true, false) => Change::SelfSigned,
        _ => Change::Nothing,
    }
}

/// Save new settings; returns what the change needs done.
pub async fn save(
    store: &dyn Storage,
    config: &HubConfig,
    new: AcmeSettings,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(AcmeSettings, Change), AcmeError> {
    let new = new.normalized();
    new.validate().map_err(AcmeError::Invalid)?;
    if new.enabled {
        check_listeners(config, new.challenge).map_err(AcmeError::Invalid)?;
    }
    let old = settings(store).await?;
    store.set_acme_settings(&new, updated_by, now).await?;
    let mut change = change(&old, &new);
    // Saved again after a failure (the port opened, the DNS fixed): try again now.
    let failing = store.acme_status().await?.is_some_and(|s| s.failures > 0);
    if change == Change::Nothing && new.enabled && failing {
        change = Change::Issue;
    }
    Ok((new.clone(), change))
}

/// The hub can answer `challenge`: it serves HTTPS itself, and HTTP-01
/// needs the plain-HTTP listener (split mode).
pub fn check_listeners(config: &HubConfig, challenge: AcmeChallenge) -> Result<(), String> {
    let tls = &config.server.tls;
    if !tls.enabled {
        return Err(
            "the hub doesn't serve HTTPS itself ([server.tls] enabled = false): the proxy in front of it handles certificates"
                .into(),
        );
    }
    if challenge == AcmeChallenge::Http && tls.https_port == config.server.port {
        return Err(
            "HTTP validation needs the hub's plain-HTTP port, and this hub listens on HTTPS only: choose TLS-ALPN"
                .into(),
        );
    }
    Ok(())
}

/// Turned off: every certificate the hub keeps issued becomes self-signed
/// again, with the same key (so the same pin). Returns whether the served
/// one changed.
pub async fn back_to_self_signed(store: &dyn Storage, config: &HubConfig) -> anyhow::Result<bool> {
    let _one_at_a_time = ISSUING.lock().await;
    let mut serving = false;
    for target in targets(store, config).await? {
        if details_of(&target.cert).is_some_and(|c| c.self_signed) {
            continue;
        }
        let key_pem = std::fs::read(&target.key)
            .with_context(|| format!("can't read {}", target.key.display()))?;
        let cert = tls::self_signed_for_key(&key_pem, &config.server.host)?;
        common::fs::write_atomic(&target.cert, &cert)
            .with_context(|| format!("can't write {}", target.cert.display()))?;
        serving |= target.serving;
    }
    Ok(serving)
}

fn info(details: CertificateDetails) -> CertificateInfo {
    let format = |at: DateTime<Utc>| at.format("%Y-%m-%d %H:%M UTC").to_string();
    CertificateInfo {
        issuer: details.issuer,
        names: details.names,
        self_signed: details.self_signed,
        not_before: format(details.not_before),
        not_after: format(details.not_after),
    }
}

/// Everything the admin page shows about the hub's certificate.
pub async fn overview(
    store: &dyn Storage,
    config: &HubConfig,
    now: DateTime<Utc>,
) -> anyhow::Result<CertificateOverview> {
    let settings = settings(store).await?;
    let status = store.acme_status().await?;
    let tls_enabled = config.server.tls.enabled;
    let targets = match tls_enabled {
        true => targets(store, config).await?,
        false => Vec::new(),
    };
    let details: Vec<_> = targets.iter().map(|t| details_of(&t.cert)).collect();
    let format = |at: DateTime<Utc>| at.format("%Y-%m-%d %H:%M UTC").to_string();
    let next_attempt_at = if settings.enabled && tls_enabled {
        let mut due: Option<DateTime<Utc>> = None;
        for cert in &details {
            let at = match cert {
                Some(c) if renewal_reason(&settings, Some(c), now).is_none() => renew_at(c),
                _ => now,
            };
            due = Some(due.map_or(at, |d| d.min(at)));
        }
        let retry = status.as_ref().filter(|s| s.failures > 0).map(retry_at);
        due.map(|at| format(retry.map_or(at, |r| r.max(at))))
    } else {
        None
    };
    let mut details = details.into_iter();
    Ok(CertificateOverview {
        tls_enabled,
        certificate: details.next().flatten().map(info),
        next_certificate: details.next().flatten().map(info),
        settings,
        next_attempt_at,
        last_attempt_at: status.as_ref().map(|s| format(s.last_attempt_at)),
        last_error: status.as_ref().and_then(|s| s.last_error.clone()),
        last_issued_at: status.and_then(|s| s.last_issued_at).map(format),
        https_port: config.server.tls.https_port,
        http_port: (config.server.tls.https_port != config.server.port)
            .then_some(config.server.port),
    })
}

/// The real authority, over the ACME protocol (RFC 8555) with instant-acme.
pub struct AcmeClient;

/// The ACME account, as kept in its file: the authority it is with, and its
/// key. A new authority gets a new account.
#[derive(serde::Serialize, serde::Deserialize)]
struct AccountFile {
    directory: String,
    credentials: instant_acme::AccountCredentials,
}

impl AcmeClient {
    fn builder(settings: &AcmeSettings) -> anyhow::Result<instant_acme::AccountBuilder> {
        tls::install_crypto_provider();
        if settings.ca_pem.is_empty() {
            return Ok(instant_acme::Account::builder()?);
        }
        // instant-acme reads a custom root from a file.
        let root = tempfile::NamedTempFile::new()?;
        std::fs::write(root.path(), &settings.ca_pem)?;
        instant_acme::Account::builder_with_root(root.path())
            .context("the authority's root certificate can't be used")
    }

    /// The account for the settings' authority: the one in the file, or a
    /// new one (then kept in the file).
    async fn account(order: &Order<'_>) -> anyhow::Result<instant_acme::Account> {
        let directory = order.settings.directory().to_string();
        if let Ok(bytes) = std::fs::read(order.account_file)
            && let Ok(file) = serde_json::from_slice::<AccountFile>(&bytes)
            && file.directory == directory
        {
            return Self::builder(order.settings)?
                .from_credentials(file.credentials)
                .await
                .context("can't use the ACME account");
        }
        let contact = format!("mailto:{}", order.settings.email);
        let contacts: Vec<&str> = match order.settings.email.is_empty() {
            true => Vec::new(),
            false => vec![contact.as_str()],
        };
        let (account, credentials) = Self::builder(order.settings)?
            .create(
                &instant_acme::NewAccount {
                    contact: &contacts,
                    terms_of_service_agreed: order.settings.terms_agreed,
                    only_return_existing: false,
                },
                directory.clone(),
                None,
            )
            .await
            .with_context(|| format!("can't open an account with {directory}"))?;
        let file = AccountFile {
            directory,
            credentials,
        };
        common::fs::write_secret_file(order.account_file, serde_json::to_vec_pretty(&file)?)
            .with_context(|| format!("can't write {}", order.account_file.display()))?;
        tracing::info!(account = %account.id(), "ACME: account created");
        Ok(account)
    }
}

#[async_trait]
impl CertificateAuthority for AcmeClient {
    async fn issue(
        &self,
        order: &Order<'_>,
        challenges: &AcmeChallenges,
    ) -> anyhow::Result<String> {
        use instant_acme::{
            AuthorizationStatus, ChallengeType, Identifier, NewOrder, OrderStatus, RetryPolicy,
        };
        let account = Self::account(order).await?;
        let identifiers: Vec<Identifier> = order
            .settings
            .domains
            .iter()
            .map(|d| Identifier::Dns(d.clone()))
            .collect();
        let mut acme_order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .context("the authority refused the order")?;
        let kind = match order.settings.challenge {
            AcmeChallenge::TlsAlpn => ChallengeType::TlsAlpn01,
            AcmeChallenge::Http => ChallengeType::Http01,
        };
        let mut authorizations = acme_order.authorizations();
        while let Some(authorization) = authorizations.next().await {
            let mut authorization = authorization?;
            match authorization.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                status => anyhow::bail!(
                    "{}: the authorization is {status:?}",
                    authorization.identifier()
                ),
            }
            let name = authorization.identifier().to_string();
            let mut challenge = authorization.challenge(kind.clone()).with_context(|| {
                format!("the authority doesn't offer {kind:?} validation for {name}")
            })?;
            let key_authorization = challenge.key_authorization();
            match order.settings.challenge {
                AcmeChallenge::Http => {
                    challenges.add_http(&challenge.token, key_authorization.as_str())
                }
                AcmeChallenge::TlsAlpn => {
                    challenges.add_tls_alpn(&name, key_authorization.digest().as_ref())?
                }
            }
            challenge.set_ready().await?;
        }

        let polling = RetryPolicy::new()
            .initial_delay(std::time::Duration::from_secs(1))
            .backoff(1.5)
            .timeout(std::time::Duration::from_secs(90));
        let where_to_look = format!(
            "it connects to port {} of each name: check the DNS, and that the port reaches the hub",
            order.settings.challenge.public_port()
        );
        let status = acme_order.poll_ready(&polling).await.with_context(|| {
            format!("the authority didn't validate the names ({where_to_look})")
        })?;
        if status != OrderStatus::Ready {
            // Say why, name by name, as the authority put it.
            let mut why = Vec::new();
            let mut authorizations = acme_order.authorizations();
            while let Some(Ok(mut authorization)) = authorizations.next().await {
                if let Ok(state) = authorization.refresh().await {
                    let name = state.identifier().to_string();
                    why.extend(state.challenges.iter().filter_map(|c| {
                        c.error.as_ref().map(|e| {
                            format!(
                                "{name}: {}",
                                e.detail.clone().unwrap_or_else(|| e.to_string())
                            )
                        })
                    }));
                }
            }
            let why = match why.is_empty() {
                true => format!("the order is {status:?}"),
                false => why.join("; "),
            };
            anyhow::bail!("the authority didn't validate the names ({where_to_look}): {why}");
        }
        acme_order
            .finalize_csr(&order.csr_der)
            .await
            .context("the authority refused the certificate request")?;
        let chain = acme_order
            .poll_certificate(&polling)
            .await
            .context("the authority didn't deliver the certificate")?;
        Ok(chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, 12, 0, 0).unwrap()
    }

    fn settings(domains: &[&str]) -> AcmeSettings {
        AcmeSettings {
            enabled: true,
            domains: domains.iter().map(|d| d.to_string()).collect(),
            terms_agreed: true,
            ..AcmeSettings::default()
        }
    }

    fn cert(names: &[&str], from: DateTime<Utc>, days: i64) -> CertificateDetails {
        CertificateDetails {
            issuer: "R10".into(),
            names: names.iter().map(|d| d.to_string()).collect(),
            self_signed: false,
            not_before: from,
            not_after: from + Duration::days(days),
        }
    }

    #[test]
    fn a_certificate_is_renewed_with_a_third_of_its_life_left() {
        let s = settings(&["hub.example.com"]);
        let issued = cert(&["hub.example.com"], at(1), 90);
        assert_eq!(renewal_reason(&s, Some(&issued), at(1)), None);
        assert_eq!(renew_at(&issued), at(1) + Duration::days(60));
        assert_eq!(
            renewal_reason(&s, Some(&issued), at(1) + Duration::days(59)),
            None
        );
        assert!(renewal_reason(&s, Some(&issued), at(1) + Duration::days(60)).is_some());

        let short = cert(&["hub.example.com"], at(1), 45);
        assert_eq!(renew_at(&short), at(1) + Duration::days(30));
    }

    #[test]
    fn a_missing_self_signed_or_misnamed_certificate_is_replaced() {
        let s = settings(&["hub.example.com", "b.example.org"]);
        assert!(renewal_reason(&s, None, at(1)).is_some());
        let mut ours = cert(&["b.example.org", "hub.example.com"], at(1), 90);
        assert_eq!(renewal_reason(&s, Some(&ours), at(1)), None, "any order");
        ours.self_signed = true;
        assert!(renewal_reason(&s, Some(&ours), at(1)).is_some());
        let other = cert(&["hub.example.com"], at(1), 90);
        assert!(renewal_reason(&s, Some(&other), at(1)).is_some());
    }

    #[test]
    fn retries_back_off_from_an_hour_to_a_day() {
        let failed = |failures| AcmeStatus {
            last_attempt_at: at(1),
            last_error: Some("refused".into()),
            failures,
            last_issued_at: None,
        };
        assert_eq!(retry_at(&failed(0)), at(1));
        assert_eq!(retry_at(&failed(1)), at(1) + Duration::hours(1));
        assert_eq!(retry_at(&failed(2)), at(1) + Duration::hours(2));
        assert_eq!(retry_at(&failed(4)), at(1) + Duration::hours(8));
        assert_eq!(retry_at(&failed(6)), at(1) + Duration::hours(24));
        assert_eq!(retry_at(&failed(60)), at(1) + Duration::hours(24));
    }

    #[test]
    fn what_a_change_of_settings_needs() {
        let off = AcmeSettings::default();
        let on = settings(&["hub.example.com"]);
        assert_eq!(change(&off, &off), Change::Nothing);
        assert_eq!(change(&off, &on), Change::Issue);
        assert_eq!(change(&on, &on), Change::Nothing);
        assert_eq!(change(&on, &off), Change::SelfSigned);

        let mut contact = on.clone();
        contact.email = "ops@example.com".into();
        assert_eq!(change(&on, &contact), Change::Nothing, "a new contact only");
        let mut renamed = on.clone();
        renamed.domains.push("b.example.org".into());
        assert_eq!(change(&on, &renamed), Change::Issue);
        let mut staging = on.clone();
        staging.authority = crate::model::AcmeAuthority::LetsEncryptStaging;
        assert_eq!(change(&on, &staging), Change::Issue);
        let mut http = on.clone();
        http.challenge = AcmeChallenge::Http;
        assert_eq!(
            change(&on, &http),
            Change::Nothing,
            "the next renewal uses it"
        );
    }
}
