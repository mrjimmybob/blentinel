//! Rotating the hub's TLS certificate and key over the protocol (M2-13,
//! proposal 0002 §4.5, D-61).
//!
//! 1. **Start:** the next key and/or certificate are made, in files beside
//!    the current ones (`crate::keys`), and recorded as `announcing`.
//! 2. **Announce:** every answer to a v7 report carries the next pins, signed
//!    with the current key; the probe holds them beside its current ones and
//!    says so in its next report.
//! 3. **Switch:** when every active probe holds them, the hub switches by
//!    itself; an admin can switch earlier ("Switch now"), cutting off the
//!    probes that aren't ready (they must enroll again). After the window
//!    (14 days by default) a rotation still waiting is flagged, not forced.
//! 4. **After:** answers tell probes still on the old pins that the hub
//!    switched; the previous key opens reports sealed for it until no probe
//!    can still hold one. Enrollment codes made before carry the old pins,
//!    so they are revoked.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use common::models::PinDigests;
use common::pins::{HubPins, Rotation, WirePins};
use common::protocol::{
    MAX_BUFFERED_REPORT_AGE_SECS, MAX_CLOCK_SKEW_SECS, ROTATION_SINCE_PROTOCOL,
};
use x25519_dalek::StaticSecret;

use crate::config::HubConfig;
use crate::keys::{Credentials, KeyFiles, pins_of};
use crate::model::{
    CompanyScope, HubKeysOverview, ProbeInfo, RotationInfo, RotationState, StartRotationRequest,
    WaitingProbe,
};
use crate::storage::{NewRotation, ProbePins, RotationRecord, Storage};

/// Rotations listed on the admin page, besides the one in progress.
const HISTORY: u32 = 5;

#[derive(Debug, thiserror::Error)]
pub enum RotationError {
    #[error("{0}")]
    Invalid(String),
    #[error("a rotation is already in progress: switch or cancel it first")]
    Busy,
    #[error("no rotation is in progress")]
    NotFound,
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

impl From<std::io::Error> for RotationError {
    fn from(e: std::io::Error) -> Self {
        Self::Failed(e.into())
    }
}

/// How long the previous key is kept after a switch: as long as a probe
/// may still deliver a report it sealed before it switched.
pub fn prev_key_retention() -> Duration {
    Duration::seconds(MAX_BUFFERED_REPORT_AGE_SECS + MAX_CLOCK_SKEW_SECS)
}

/// The rotation in progress, if any.
pub async fn active(store: &dyn Storage) -> anyhow::Result<Option<RotationRecord>> {
    Ok(store
        .rotations(1)
        .await?
        .into_iter()
        .next()
        .filter(|r| r.state == RotationState::Announcing))
}

/// The pin of the certificate the hub serves now; `None` without HTTPS.
pub fn current_tls_spki(config: &HubConfig) -> anyhow::Result<Option<[u8; 32]>> {
    if !config.server.tls.enabled {
        return Ok(None);
    }
    tls_spki_of(&config.server.resolved_tls_cert_path()).map(Some)
}

fn tls_spki_of(cert: &Path) -> anyhow::Result<[u8; 32]> {
    use anyhow::Context;
    let pem = std::fs::read(cert).with_context(|| format!("can't read {}", cert.display()))?;
    crate::tls::cert_spki_sha256(&pem)
}

fn load_secret(path: &Path) -> anyhow::Result<StaticSecret> {
    use anyhow::Context;
    let bytes =
        common::fs::load_key(path).with_context(|| format!("can't read {}", path.display()))?;
    Ok(StaticSecret::from(bytes))
}

/// What the hub tells probes now: its pins, the next ones while a rotation
/// announces, and the previous key while reports may still be sealed for it.
pub async fn credentials(
    store: &dyn Storage,
    config: &HubConfig,
    hub_secret: &StaticSecret,
) -> anyhow::Result<Credentials> {
    let files = KeyFiles::of(config);
    let current_tls = current_tls_spki(config)?;
    let mut credentials = Credentials::current_only(hub_secret, current_tls);
    if let Some(rotation) = active(store).await? {
        let next_secret = match rotation.rotates_key {
            true => load_secret(&files.identity_next)?,
            false => hub_secret.clone(),
        };
        let next_tls = match rotation.rotates_tls {
            true => Some(tls_spki_of(&files.tls_cert_next)?),
            false => current_tls,
        };
        credentials.next = Some(pins_of(&next_secret, next_tls));
    }
    if files.identity_prev.exists() {
        credentials.prev_secret = Some(load_secret(&files.identity_prev)?);
    }
    Ok(credentials)
}

/// Start a rotation: make the next key and/or certificate and announce them.
pub async fn start(
    store: &dyn Storage,
    config: &HubConfig,
    request: &StartRotationRequest,
    started_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<RotationRecord, RotationError> {
    request.validate().map_err(RotationError::Invalid)?;
    if request.tls && !config.server.tls.enabled {
        return Err(RotationError::Invalid(
            "the hub doesn't serve HTTPS itself: it has no certificate to rotate".into(),
        ));
    }
    if active(store).await?.is_some() {
        return Err(RotationError::Busy);
    }
    let files = KeyFiles::of(config);
    if request.key {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        common::fs::write_secret_file(&files.identity_next, secret.as_bytes())?;
    }
    if request.tls {
        crate::tls::generate_self_signed_cert(
            &files.tls_cert_next.to_string_lossy(),
            &files.tls_key_next.to_string_lossy(),
            &config.server.host,
        )?;
    }
    let id = store
        .start_rotation(&NewRotation {
            rotates_tls: request.tls,
            rotates_key: request.key,
            window_days: request.window_days,
            started_by: started_by.map(str::to_string),
            started_at: now,
            deadline_at: now + Duration::days(i64::from(request.window_days)),
        })
        .await?;
    tracing::info!(
        rotation = id,
        tls = request.tls,
        key = request.key,
        "Rotation started: probes get the next pins with their next report"
    );
    find(store, id).await
}

async fn find(store: &dyn Storage, id: i64) -> Result<RotationRecord, RotationError> {
    store
        .rotations(HISTORY + 1)
        .await?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or(RotationError::NotFound)
}

/// Cancel the rotation in progress: the next files go, nothing changed.
pub async fn cancel(
    store: &dyn Storage,
    config: &HubConfig,
    ended_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<RotationRecord, RotationError> {
    let rotation = active(store).await?.ok_or(RotationError::NotFound)?;
    if !store
        .end_rotation(rotation.id, RotationState::Cancelled, ended_by, now)
        .await?
    {
        return Err(RotationError::NotFound);
    }
    let files = KeyFiles::of(config);
    for path in [files.identity_next, files.tls_cert_next, files.tls_key_next] {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                tracing::warn!("Can't delete {}: {e}", path.display())
            }
            _ => {}
        }
    }
    tracing::info!(rotation = rotation.id, "Rotation cancelled");
    find(store, rotation.id).await
}

/// What a switch changed, for the running hub to follow.
pub struct Switched {
    pub rotation: RotationRecord,
    /// The new key, if the key rotated.
    pub new_secret: Option<StaticSecret>,
    /// The certificate files changed: the listener must load them.
    pub tls_changed: bool,
    /// Enrollment codes revoked: they carried the old pins.
    pub codes_revoked: usize,
}

/// Switch to the next key and/or certificate.
pub async fn switch(
    store: &dyn Storage,
    config: &HubConfig,
    ended_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Switched, RotationError> {
    let rotation = active(store).await?.ok_or(RotationError::NotFound)?;
    let files = KeyFiles::of(config);
    switch_files(&rotation, &files)?;
    store
        .end_rotation(rotation.id, RotationState::Switched, ended_by, now)
        .await?;
    let new_secret = match rotation.rotates_key {
        true => Some(load_secret(&files.identity)?),
        false => None,
    };
    let mut codes_revoked = 0;
    for code in store.enrollment_tokens(&CompanyScope::All, now).await? {
        if store.revoke_enrollment_token(&code.id, now).await? {
            codes_revoked += 1;
        }
    }
    tracing::info!(
        rotation = rotation.id,
        codes_revoked,
        "Rotation switched: the hub uses its new pins"
    );
    Ok(Switched {
        rotation: find(store, rotation.id).await?,
        new_secret,
        tls_changed: rotation.rotates_tls,
        codes_revoked,
    })
}

/// Move the next files in place. Each step is skipped if done already, so
/// a switch interrupted by a crash can be finished ([`recover`]).
fn switch_files(rotation: &RotationRecord, files: &KeyFiles) -> anyhow::Result<()> {
    if rotation.rotates_key && files.identity_next.exists() {
        let current = common::fs::load_key(&files.identity)?;
        common::fs::write_secret_file(&files.identity_prev, current)?;
        std::fs::rename(&files.identity_next, &files.identity)?; // replaces atomically
        let public = x25519_dalek::PublicKey::from(&load_secret(&files.identity)?);
        crate::identity::write_pub_key_file(&files.identity.with_extension("pub"), &public);
    }
    if rotation.rotates_tls {
        if files.tls_key_next.exists() {
            std::fs::rename(&files.tls_key_next, &files.tls_key)?;
        }
        if files.tls_cert_next.exists() {
            std::fs::rename(&files.tls_cert_next, &files.tls_cert)?;
        }
    }
    Ok(())
}

/// At startup: a rotation still `announcing` whose next files are partly
/// gone was switching when the hub stopped. Finish it, so the files and the
/// record agree. `true` if it did.
pub async fn recover(
    store: &dyn Storage,
    config: &HubConfig,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let Some(rotation) = active(store).await? else {
        return Ok(false);
    };
    let files = KeyFiles::of(config);
    let key_moved = rotation.rotates_key && !files.identity_next.exists();
    let tls_moved =
        rotation.rotates_tls && !(files.tls_cert_next.exists() && files.tls_key_next.exists());
    if !(key_moved || tls_moved) {
        return Ok(false);
    }
    tracing::warn!(
        rotation = rotation.id,
        "A switch was interrupted; finishing it"
    );
    switch_files(&rotation, &files)?;
    store
        .end_rotation(rotation.id, RotationState::Switched, None, now)
        .await?;
    Ok(true)
}

/// Does `probe` hold `next`? Yes if it reported them as its next pins, or as
/// its current ones (a probe without a certificate pin, when only the
/// certificate rotates, has nothing to take).
pub fn is_ready(next: &HubPins, probe: &ProbePins) -> bool {
    [&probe.next, &probe.current]
        .into_iter()
        .flatten()
        .any(|digest| next.matches(digest))
}

/// What the answer to a report says about the hub's pins, if anything: the
/// next ones while a rotation announces (to a probe that doesn't hold them
/// yet), or that the hub switched (to a probe still on other pins).
pub fn for_probe(
    credentials: &Credentials,
    pins: Option<&PinDigests>,
    protocol_version: u16,
) -> Option<Rotation> {
    if protocol_version < ROTATION_SINCE_PROTOCOL {
        return None;
    }
    let holds = |hub: &HubPins, digest: Option<&String>| digest.is_some_and(|d| hub.matches(d));
    if let Some(next) = &credentials.next {
        if holds(next, pins.and_then(|p| p.next.as_ref())) || holds(next, pins.map(|p| &p.current))
        {
            return None;
        }
        return Some(Rotation {
            pins: WirePins::from(next),
            switched: false,
        });
    }
    if holds(&credentials.current, pins.map(|p| &p.current)) {
        return None;
    }
    Some(Rotation {
        pins: WirePins::from(&credentials.current),
        switched: true,
    })
}

/// What [`tick`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    Nothing,
    /// Every active probe holds the next pins: switch.
    AllReady,
    /// The window passed with `waiting` probes not ready (said once).
    Overdue {
        waiting: usize,
    },
}

/// Once a minute: is the rotation ready, or overdue? And the previous key,
/// once no probe can still hold a report sealed for it, is deleted.
pub async fn tick(
    store: &dyn Storage,
    config: &HubConfig,
    credentials: &Credentials,
    now: DateTime<Utc>,
) -> anyhow::Result<Tick> {
    prune_prev_key(store, config, now).await?;
    let (Some(rotation), Some(next)) = (active(store).await?, &credentials.next) else {
        return Ok(Tick::Nothing);
    };
    let pins = store.active_probe_pins().await?;
    let waiting = pins.iter().filter(|p| !is_ready(next, p)).count();
    if waiting == 0 {
        return Ok(Tick::AllReady);
    }
    if now >= rotation.deadline_at && !rotation.deadline_warned {
        store.mark_rotation_warned(rotation.id).await?;
        return Ok(Tick::Overdue { waiting });
    }
    Ok(Tick::Nothing)
}

async fn prune_prev_key(
    store: &dyn Storage,
    config: &HubConfig,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let files = KeyFiles::of(config);
    if !files.identity_prev.exists() {
        return Ok(());
    }
    let last_key_switch = store
        .rotations(HISTORY + 1)
        .await?
        .into_iter()
        .filter(|r| r.state == RotationState::Switched && r.rotates_key)
        .find_map(|r| r.ended_at);
    if last_key_switch.is_some_and(|at| now - at > prev_key_retention()) {
        std::fs::remove_file(&files.identity_prev)?;
        tracing::info!("The previous hub key is deleted: no probe can still use it");
    }
    Ok(())
}

/// Everything the admin page shows about the hub's keys.
pub async fn overview(
    store: &dyn Storage,
    config: &HubConfig,
    credentials: &Credentials,
    now: DateTime<Utc>,
) -> anyhow::Result<HubKeysOverview> {
    let rotations = store.rotations(HISTORY + 1).await?;
    let tls_expires = if config.server.tls.enabled {
        std::fs::read(config.server.resolved_tls_cert_path())
            .ok()
            .and_then(|pem| crate::tls::cert_not_after(&pem).ok())
            .map(|at| format!("{}", at.date()))
    } else {
        None
    };
    let mut rotation = None;
    let mut history = Vec::new();
    for record in rotations {
        if record.state == RotationState::Announcing {
            let (ready, waiting) = match &credentials.next {
                Some(next) => readiness(store, next).await?,
                None => (0, Vec::new()),
            };
            rotation = Some(info(&record, now, ready, waiting));
        } else if history.len() < HISTORY as usize {
            history.push(info(&record, now, 0, Vec::new()));
        }
    }
    Ok(HubKeysOverview {
        hub_key: hex::encode(credentials.current.hub_key),
        tls_enabled: config.server.tls.enabled,
        tls_spki_sha256: credentials.current.tls_spki_sha256.map(hex::encode),
        tls_expires,
        rotation,
        history,
    })
}

/// How many active probes hold `next`, and the ones that don't, with why.
async fn readiness(
    store: &dyn Storage,
    next: &HubPins,
) -> anyhow::Result<(u32, Vec<WaitingProbe>)> {
    let pins = store.active_probe_pins().await?;
    let probes: Vec<ProbeInfo> = store.probe_records(&CompanyScope::All).await?;
    let mut ready = 0;
    let mut waiting = Vec::new();
    for probe in &pins {
        if is_ready(next, probe) {
            ready += 1;
            continue;
        }
        let Some(info) = probes.iter().find(|p| p.id == probe.probe_id) else {
            continue;
        };
        let reason = if probe.current.is_none() {
            format!(
                "runs an older probe ({}): it takes new keys once it updates itself",
                info.probe_version.as_deref().unwrap_or("version unknown")
            )
        } else {
            "waiting for its next report".to_string()
        };
        waiting.push(WaitingProbe {
            id: info.id.clone(),
            name: info.name.clone(),
            company_name: info.company_name.clone(),
            probe_version: info.probe_version.clone(),
            last_seen_at: info.last_seen_at.clone(),
            reason,
        });
    }
    Ok((ready, waiting))
}

fn info(
    record: &RotationRecord,
    now: DateTime<Utc>,
    ready: u32,
    waiting: Vec<WaitingProbe>,
) -> RotationInfo {
    let format = |at: DateTime<Utc>| at.format("%Y-%m-%d %H:%M UTC").to_string();
    RotationInfo {
        id: record.id,
        rotates_tls: record.rotates_tls,
        rotates_key: record.rotates_key,
        window_days: record.window_days,
        state: record.state,
        started_by: record.started_by.clone(),
        started_at: format(record.started_at),
        deadline_at: format(record.deadline_at),
        ended_by: record.ended_by.clone(),
        ended_at: record.ended_at.map(format),
        overdue: record.state == RotationState::Announcing && now >= record.deadline_at,
        ready,
        waiting,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub(key: u8, tls: Option<u8>) -> HubPins {
        HubPins {
            hub_key: [key; 32],
            signing_key: [key + 1; 32],
            tls_spki_sha256: tls.map(|t| [t; 32]),
        }
    }

    fn probe(current: Option<HubPins>, next: Option<HubPins>) -> ProbePins {
        ProbePins {
            probe_id: "p".into(),
            current: current.map(|p| p.digest()),
            next: next.map(|p| p.digest()),
        }
    }

    fn digests(current: HubPins, next: Option<HubPins>) -> PinDigests {
        PinDigests {
            current: current.digest(),
            next: next.map(|p| p.digest()),
        }
    }

    #[test]
    fn a_probe_is_ready_once_it_holds_the_next_pins() {
        let (old, new) = (hub(1, Some(3)), hub(7, Some(9)));
        assert!(!is_ready(&new, &probe(Some(old), None)));
        assert!(is_ready(&new, &probe(Some(old), Some(new))));
        assert!(
            !is_ready(&new, &probe(None, None)),
            "a v6 probe reports no pins"
        );
    }

    #[test]
    fn a_probe_without_a_certificate_pin_is_ready_for_a_certificate_rotation() {
        let (old, new) = (hub(1, Some(3)), hub(1, Some(9))); // only the certificate
        assert!(is_ready(&new, &probe(Some(old.without_tls()), None)));
        assert!(!is_ready(&new, &probe(Some(old), None)));
    }

    #[test]
    fn answers_announce_until_the_probe_holds_them() {
        let (old, new) = (hub(1, Some(3)), hub(7, Some(9)));
        let credentials = Credentials {
            current: old,
            next: Some(new),
            prev_secret: None,
        };
        let offered = for_probe(&credentials, Some(&digests(old, None)), 7).unwrap();
        assert!(!offered.switched);
        assert_eq!(HubPins::try_from(&offered.pins).unwrap(), new);
        assert_eq!(
            for_probe(&credentials, Some(&digests(old, Some(new))), 7),
            None
        );
        assert_eq!(
            for_probe(&credentials, Some(&digests(old, None)), 6),
            None,
            "v6"
        );
    }

    #[test]
    fn after_a_switch_answers_tell_probes_still_on_the_old_pins() {
        let (old, new) = (hub(1, Some(3)), hub(7, Some(9)));
        let credentials = Credentials {
            current: new,
            next: None,
            prev_secret: None,
        };
        let told = for_probe(&credentials, Some(&digests(old, Some(new))), 7).unwrap();
        assert!(told.switched);
        assert_eq!(HubPins::try_from(&told.pins).unwrap(), new);
        assert_eq!(for_probe(&credentials, Some(&digests(new, None)), 7), None);
        assert_eq!(
            for_probe(&credentials, Some(&digests(new.without_tls(), None)), 7),
            None,
            "a probe over plain HTTP"
        );
    }
}
