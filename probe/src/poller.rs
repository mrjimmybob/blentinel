//! Asking the hub for commands between reports (M4-03, D-76; the messages:
//! `common::poll`). A task of its own, beside the monitoring loop: the
//! report cycle doesn't change.
//!
//! It asks, hands any commands to the monitoring loop (which runs them and
//! reports at once), and asks again when the hub's answer says: at once
//! after a held poll, later if the hub prefers. Whatever goes wrong (no
//! answer, an error, an answer it can't trust), it asks again in
//! `FALLBACK_POLL_SECS`; a hub that doesn't take polls (an older one), or
//! says not to, is asked again in `IDLE_POLL_SECS`. Commands also come with
//! every report, so a failing poll only makes them slower.

use std::time::Duration;

use chrono::Utc;
use common::commands::Command;
use common::poll::{
    CommandPoll, FALLBACK_POLL_SECS, IDLE_POLL_SECS, PollAnswer, open_answer, seal_poll,
};
use ed25519_dalek::SigningKey;
use reqwest::StatusCode;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::enrollment::Enrollment;
use crate::transport::{HubAnswer, HubTransport, TransportError};

/// The hub to ask: what the monitoring loop reports to.
#[derive(Clone)]
pub struct Target {
    /// To notice a change (a new enrollment, a rotation).
    pub enrollment: Enrollment,
    pub transport: HubTransport,
    pub hub_key: [u8; 32],
    pub hub_signing_keys: Vec<[u8; 32]>,
}

/// The shortest wait between two polls, whatever the hub says: a hub that
/// answers "ask again now" at once must not make the probe spin.
const MIN_GAP: Duration = Duration::from_secs(1);

/// What came of one poll.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub commands: Vec<Command>,
    /// When to ask again.
    pub next: Duration,
}

fn secs(secs: u32) -> Duration {
    Duration::from_secs(u64::from(secs))
}

/// Decide from the hub's answer; `open` opens and checks a sealed answer.
pub fn decide(
    answer: Result<HubAnswer, TransportError>,
    open: impl FnOnce(&[u8]) -> Result<PollAnswer, common::ack::AckError>,
) -> Outcome {
    let nothing = |next: u32| Outcome {
        commands: Vec::new(),
        next: secs(next),
    };
    let answer = match answer {
        Ok(answer) => answer,
        Err(e) => {
            debug!("Command poll failed: {e}");
            return nothing(FALLBACK_POLL_SECS);
        }
    };
    match answer.status {
        StatusCode::OK if answer.is_ack => match open(&answer.body) {
            Ok(opened) => Outcome {
                commands: opened.commands,
                next: opened
                    .next_poll_secs
                    .map_or(secs(IDLE_POLL_SECS), secs)
                    .max(MIN_GAP),
            },
            Err(e) => {
                warn!("The hub's answer to a command poll can't be trusted: {e}");
                nothing(FALLBACK_POLL_SECS)
            }
        },
        // A hub from before M4-03, or one that turned polls away.
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::FORBIDDEN => {
            nothing(IDLE_POLL_SECS)
        }
        status => {
            debug!("The hub answered a command poll with {status}");
            nothing(FALLBACK_POLL_SECS)
        }
    }
}

/// Start asking. The task asks the hub in `target` (none: not enrolled, it
/// waits), sends the commands it gets on `commands`, and stops when the
/// handle is aborted or the loop stops listening.
pub fn spawn(
    probe_key: SigningKey,
    mut target: watch::Receiver<Option<Target>>,
    commands: mpsc::Sender<Vec<Command>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let current = target.borrow_and_update().clone();
            let Some(current) = current else {
                if target.changed().await.is_err() {
                    return;
                }
                continue;
            };
            let next = match seal_poll(&CommandPoll::new(&probe_key, Utc::now()), &current.hub_key)
            {
                Ok(sealed) => {
                    let answer = current.transport.poll_commands(&sealed).await;
                    let outcome = decide(answer, |body| {
                        open_answer(body, &probe_key, &current.hub_signing_keys, &sealed)
                    });
                    if !outcome.commands.is_empty()
                        && commands.send(outcome.commands).await.is_err()
                    {
                        return; // the monitoring loop is gone
                    }
                    outcome.next
                }
                Err(e) => {
                    warn!("Can't seal a command poll: {e}");
                    secs(FALLBACK_POLL_SECS)
                }
            };
            // A new hub (or keys) is asked at once.
            tokio::select! {
                _ = tokio::time::sleep(next) => {}
                changed = target.changed() => if changed.is_err() { return },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::ack::AckError;
    use common::commands::CommandSpec;

    fn reply(code: u16, is_ack: bool) -> Result<HubAnswer, TransportError> {
        Ok(HubAnswer {
            status: StatusCode::from_u16(code).unwrap(),
            body: Vec::new(),
            is_ack,
        })
    }

    fn opened(commands: Vec<Command>, next: Option<u32>) -> PollAnswer {
        PollAnswer {
            poll_sha256: String::new(),
            hub_time: Utc::now(),
            commands,
            next_poll_secs: next,
        }
    }

    #[test]
    fn the_hub_sets_the_pace() {
        let run = Command {
            id: "c1".into(),
            spec: CommandSpec::RunChecks,
        };
        let outcome = decide(reply(200, true), |_| Ok(opened(vec![run.clone()], Some(0))));
        assert_eq!(outcome.commands, [run]);
        assert_eq!(outcome.next, MIN_GAP, "at once, but never spinning");
        let later = decide(reply(200, true), |_| Ok(opened(vec![], Some(30))));
        assert_eq!(later.next, Duration::from_secs(30));
        let stop = decide(reply(200, true), |_| Ok(opened(vec![], None)));
        assert_eq!(stop.next, secs(IDLE_POLL_SECS));
    }

    #[test]
    fn failures_fall_back_to_slow_polling_and_old_hubs_to_idle() {
        let fallback = secs(FALLBACK_POLL_SECS);
        let idle = secs(IDLE_POLL_SECS);
        let untrusted = |_: &[u8]| Err(AckError::BadSignature);
        assert_eq!(decide(reply(200, true), untrusted).next, fallback);
        assert_eq!(
            decide(Err(TransportError::Timeout), untrusted).next,
            fallback
        );
        assert_eq!(decide(reply(500, false), untrusted).next, fallback);
        assert_eq!(decide(reply(502, false), untrusted).next, fallback);
        assert_eq!(
            decide(reply(200, false), untrusted).next,
            fallback,
            "a proxy's page"
        );
        for old_or_refused in [404, 405, 403] {
            assert_eq!(decide(reply(old_or_refused, false), untrusted).next, idle);
        }
        assert!(decide(reply(200, true), untrusted).commands.is_empty());
    }
}
