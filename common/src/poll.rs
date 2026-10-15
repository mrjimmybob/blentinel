//! Asking for commands between reports (M4-03, proposal 0003 §4.3, D-76).
//!
//! Commands ride in the answer to every report (D-56), so they wait up to a
//! report interval. To act sooner, the probe also asks the hub, on its own
//! loop, whether commands are waiting: `POST /api/probe/commands` with a
//! [`CommandPoll`], signed with its key and sealed for the hub like a
//! report. Still probe-initiated (FP-1).
//!
//! The hub answers with a [`PollAnswer`], signed with its key and sealed for
//! the probe like an ack (D-48), bound to the poll. **The hub sets the
//! pace:** it may hold the poll open until a command is queued (up to
//! [`HOLD_SECS`]), and says when to ask again (`next_poll_secs`). So the hub
//! can answer at once (polling), hold (near-instant commands), or stop the
//! probe asking, without changing the probe.
//!
//! Both signatures are domain separated: a poll can't pass for a report,
//! nor an answer for an ack, though the same keys sign them.

use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use x25519_dalek::StaticSecret;

use crate::ack::{AckError, open_signed, seal_signed};
use crate::commands::Command;
use crate::crypto::{self, CryptoError};
use crate::protocol::{MAX_CLOCK_SKEW_SECS, MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION};

/// Where probes ask.
pub const POLL_PATH: &str = "/api/probe/commands";
/// The longest the hub holds a poll open: under the 30 s after which many
/// proxies drop a quiet connection.
pub const HOLD_SECS: u32 = 25;
/// How long a probe waits before asking again after a failed poll (no
/// answer, an error, one it can't trust): the slowest it learns of commands
/// while the hub is reachable.
pub const FALLBACK_POLL_SECS: u32 = 30;
/// How long a probe waits for an answer: a held poll plus a margin.
pub const POLL_TIMEOUT_SECS: u32 = HOLD_SECS + 20;
/// How long a probe waits before asking again a hub that doesn't take polls
/// (an older hub), or says not to poll: it may later.
pub const IDLE_POLL_SECS: u32 = 60 * 60;

const POLL_DOMAIN: &[u8] = b"blentinel command poll v1\n";
const ANSWER_DOMAIN: &[u8] = b"blentinel command poll answer v1\n";

/// A probe asking for its commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPoll {
    pub protocol_version: u16,
    /// Hex of the probe's verifying key; the signature must be by it.
    pub probe_id: String,
    /// When it asked: the hub refuses a poll more than the allowed clock
    /// skew away from its own clock (a replayed one, mostly).
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Serialize)]
struct Signable<'a> {
    protocol_version: u16,
    probe_id: &'a str,
    timestamp: DateTime<Utc>,
}

impl CommandPoll {
    fn signed_bytes(&self) -> Vec<u8> {
        let signable = Signable {
            protocol_version: self.protocol_version,
            probe_id: &self.probe_id,
            timestamp: self.timestamp,
        };
        let json = serde_json::to_vec(&signable).expect("a poll always serializes");
        [POLL_DOMAIN, &json].concat()
    }

    /// A poll by `probe_key` at `now`, signed.
    pub fn new(probe_key: &SigningKey, now: DateTime<Utc>) -> Self {
        let mut poll = Self {
            protocol_version: PROTOCOL_VERSION,
            probe_id: hex::encode(probe_key.verifying_key().as_bytes()),
            timestamp: now,
            signature: None,
        };
        poll.signature = Some(crypto::sign(&poll.signed_bytes(), probe_key));
        poll
    }
}

/// Why a poll was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PollError {
    #[error("the poll can't be opened (not sealed for this hub)")]
    Undecryptable,
    #[error("not a command poll")]
    NotAPoll,
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("the poll isn't signed by the probe it names")]
    BadSignature,
    #[error("the poll's time is more than {MAX_CLOCK_SKEW_SECS} s from the hub's")]
    BadTime,
}

/// Probe side: seal a poll for the hub with key `hub_key`.
pub fn seal_poll(poll: &CommandPoll, hub_key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    let json = serde_json::to_vec(poll).expect("a poll always serializes");
    crypto::seal_for_hub(&json, hub_key)
}

/// Hub side: open a poll with one of `hub_secrets` (the current key, then
/// the previous one after a switch, D-61), and check its version, signature
/// and time against `now`.
pub fn open_poll(
    sealed: &[u8],
    hub_secrets: &[&StaticSecret],
    now: DateTime<Utc>,
) -> Result<CommandPoll, PollError> {
    let json = hub_secrets
        .iter()
        .find_map(|secret| crypto::open_from_probe(sealed, secret).ok())
        .ok_or(PollError::Undecryptable)?;
    let poll: CommandPoll = serde_json::from_slice(&json).map_err(|_| PollError::NotAPoll)?;
    if !(MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&poll.protocol_version) {
        return Err(PollError::UnsupportedVersion(poll.protocol_version));
    }
    let signature = poll.signature.as_deref().ok_or(PollError::BadSignature)?;
    crypto::verify(&poll.signed_bytes(), signature, &poll.probe_id)
        .map_err(|_| PollError::BadSignature)?;
    if (now - poll.timestamp).abs() > Duration::seconds(MAX_CLOCK_SKEW_SECS) {
        return Err(PollError::BadTime);
    }
    Ok(poll)
}

/// The hub's answer to a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollAnswer {
    /// SHA-256 (hex) of the sealed poll this answers.
    pub poll_sha256: String,
    pub hub_time: DateTime<Utc>,
    /// Commands waiting for the probe (sent with reports too, until done).
    #[serde(default)]
    pub commands: Vec<Command>,
    /// When to ask again: `Some(0)` at once, `Some(n)` in `n` seconds,
    /// `None` not now (the probe asks again after [`IDLE_POLL_SECS`]).
    #[serde(default)]
    pub next_poll_secs: Option<u32>,
}

/// Hub side: sign `answer` and seal it for `probe_id`.
pub fn seal_answer(
    answer: &PollAnswer,
    hub_secret: &StaticSecret,
    probe_id: &str,
) -> Result<Vec<u8>, CryptoError> {
    seal_signed(answer, ANSWER_DOMAIN, hub_secret, probe_id)
}

/// Probe side: open an answer, check it is signed by one of
/// `hub_signing_keys` and answers `sealed_poll`.
pub fn open_answer(
    body: &[u8],
    probe_key: &SigningKey,
    hub_signing_keys: &[[u8; 32]],
    sealed_poll: &[u8],
) -> Result<PollAnswer, AckError> {
    let answer: PollAnswer = open_signed(body, ANSWER_DOMAIN, probe_key, hub_signing_keys)?;
    if answer.poll_sha256 != crate::ack::report_sha256(sealed_poll) {
        return Err(AckError::WrongReport);
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandSpec;
    use rand::rngs::OsRng;

    fn now() -> DateTime<Utc> {
        "2026-10-09T12:00:00Z".parse().unwrap()
    }

    struct Keys {
        hub: StaticSecret,
        hub_public: [u8; 32],
        hub_signing: [u8; 32],
        probe: SigningKey,
    }

    fn keys() -> Keys {
        let hub = StaticSecret::random_from_rng(OsRng);
        Keys {
            hub_public: x25519_dalek::PublicKey::from(&hub).to_bytes(),
            hub_signing: crypto::hub_signing_key(&hub).verifying_key().to_bytes(),
            probe: SigningKey::generate(&mut OsRng),
            hub,
        }
    }

    #[test]
    fn a_poll_opens_on_the_hub_and_its_answer_on_the_probe() {
        let k = keys();
        let sealed = seal_poll(&CommandPoll::new(&k.probe, now()), &k.hub_public).unwrap();
        let poll = open_poll(&sealed, &[&k.hub], now() + Duration::seconds(3)).unwrap();
        assert_eq!(
            poll.probe_id,
            hex::encode(k.probe.verifying_key().as_bytes())
        );

        let answer = PollAnswer {
            poll_sha256: crate::ack::report_sha256(&sealed),
            hub_time: now(),
            commands: vec![Command {
                id: "c1".into(),
                spec: CommandSpec::RunChecks,
            }],
            next_poll_secs: Some(0),
        };
        let body = seal_answer(&answer, &k.hub, &poll.probe_id).unwrap();
        assert_eq!(
            open_answer(&body, &k.probe, &[k.hub_signing], &sealed).unwrap(),
            answer
        );
        // Bound to its poll.
        let other = seal_poll(&CommandPoll::new(&k.probe, now()), &k.hub_public).unwrap();
        assert!(matches!(
            open_answer(&body, &k.probe, &[k.hub_signing], &other),
            Err(AckError::WrongReport)
        ));
    }

    #[test]
    fn forged_stale_or_foreign_polls_are_refused() {
        let k = keys();
        let poll = CommandPoll::new(&k.probe, now());
        let sealed = seal_poll(&poll, &k.hub_public).unwrap();
        assert_eq!(
            open_poll(&sealed, &[&k.hub], now() + Duration::minutes(6)),
            Err(PollError::BadTime)
        );
        assert_eq!(
            open_poll(&sealed, &[&k.hub], now() - Duration::minutes(6)),
            Err(PollError::BadTime)
        );
        let other_hub = StaticSecret::random_from_rng(OsRng);
        assert_eq!(
            open_poll(&sealed, &[&other_hub], now()),
            Err(PollError::Undecryptable)
        );
        // Another probe's ID on this probe's signature.
        let mut forged = poll.clone();
        forged.probe_id = hex::encode(SigningKey::generate(&mut OsRng).verifying_key().as_bytes());
        let sealed = seal_poll(&forged, &k.hub_public).unwrap();
        assert_eq!(
            open_poll(&sealed, &[&k.hub], now()),
            Err(PollError::BadSignature)
        );
        let mut unsigned = poll;
        unsigned.signature = None;
        let sealed = seal_poll(&unsigned, &k.hub_public).unwrap();
        assert_eq!(
            open_poll(&sealed, &[&k.hub], now()),
            Err(PollError::BadSignature)
        );
    }

    #[test]
    fn an_ack_signature_never_passes_for_an_answer() {
        // The hub signs acks and poll answers with the same key; an answer
        // made as an ack (no domain) must not open as an answer.
        let k = keys();
        let sealed = seal_poll(&CommandPoll::new(&k.probe, now()), &k.hub_public).unwrap();
        let answer = PollAnswer {
            poll_sha256: crate::ack::report_sha256(&sealed),
            hub_time: now(),
            commands: vec![],
            next_poll_secs: None,
        };
        let probe_id = hex::encode(k.probe.verifying_key().as_bytes());
        let as_ack = seal_signed(&answer, b"", &k.hub, &probe_id).unwrap();
        assert!(matches!(
            open_answer(&as_ack, &k.probe, &[k.hub_signing], &sealed),
            Err(AckError::BadSignature)
        ));
    }
}
