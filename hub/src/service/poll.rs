//! Answering a probe that asks for its commands between reports (M4-03,
//! proposal 0003 §4.3, D-76; the message itself: `common::poll`).
//!
//! The hub **holds** a poll open until a command is queued for that probe,
//! or [`common::poll::HOLD_SECS`] pass, then answers with what is waiting
//! and "ask again now". So "Run checks now" reaches a probe in about a
//! second, still over a connection the probe opened (FP-1). If too many
//! polls are held, it answers at once and says to ask again in
//! [`common::poll::FALLBACK_POLL_SECS`]: polling, the slow but sure path.
//!
//! A probe holds one poll at a time: a new one from the same probe releases
//! the one before (answered with nothing; the probe has moved on).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use common::commands::Command;
use common::poll::{CommandPoll, FALLBACK_POLL_SECS, PollError};
use tokio::sync::Notify;
use x25519_dalek::StaticSecret;

use crate::service::commands;
use crate::storage::Storage;

/// The most polls held at once; beyond it, polls are answered at once.
pub const MAX_HELD: usize = 10_000;

/// The polls being held, by probe, and how to wake them.
pub struct CommandWaiters {
    hold: Duration,
    held: Mutex<HashMap<String, Waiter>>,
}

struct Waiter {
    /// Which poll of the probe this is (a newer one replaces it).
    generation: u64,
    wake: std::sync::Arc<Notify>,
}

/// How a hold ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// Woken: a command was queued, or a newer poll replaced this one.
    Woken,
    /// Nothing happened for the whole hold.
    TimedOut,
    /// Not held: too many polls are held already.
    Full,
}

impl CommandWaiters {
    /// Holding polls up to `hold` (the protocol's [`common::poll::HOLD_SECS`];
    /// tests use less).
    pub fn new(hold: Duration) -> Self {
        Self {
            hold,
            held: Mutex::default(),
        }
    }

    /// Hold the poll of `probe_id` until woken or the hold is over.
    pub async fn hold(&self, probe_id: &str) -> Held {
        let wake = std::sync::Arc::new(Notify::new());
        let generation = {
            let mut held = self.held.lock().expect("waiters lock");
            let generation = match held.get(probe_id) {
                Some(previous) => {
                    // Its earlier poll is released: one per probe.
                    previous.wake.notify_one();
                    previous.generation + 1
                }
                None if held.len() >= MAX_HELD => return Held::Full,
                None => 0,
            };
            held.insert(
                probe_id.to_string(),
                Waiter {
                    generation,
                    wake: wake.clone(),
                },
            );
            generation
        };
        let outcome = tokio::select! {
            _ = wake.notified() => Held::Woken,
            _ = tokio::time::sleep(self.hold) => Held::TimedOut,
        };
        let mut held = self.held.lock().expect("waiters lock");
        if held
            .get(probe_id)
            .is_some_and(|w| w.generation == generation)
        {
            held.remove(probe_id);
        }
        outcome
    }

    /// A command was queued for `probe_id`: answer its held poll now.
    pub fn wake(&self, probe_id: &str) {
        if let Some(waiter) = self.held.lock().expect("waiters lock").get(probe_id) {
            // A permit is kept if the poll isn't waiting yet, so a command
            // queued in that instant isn't missed.
            waiter.wake.notify_one();
        }
    }

    /// How many polls are held.
    pub fn held(&self) -> usize {
        self.held.lock().expect("waiters lock").len()
    }
}

/// Why a poll gets no answer.
#[derive(Debug, thiserror::Error)]
pub enum PollRefusal {
    #[error(transparent)]
    Poll(#[from] PollError),
    #[error("poll from unregistered probe {0}")]
    UnknownProbe(String),
    #[error("poll from revoked probe {0}")]
    Revoked(String),
    #[error("storage failure: {0:#}")]
    Storage(#[source] anyhow::Error),
}

/// What to answer a poll with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub commands: Vec<Command>,
    pub next_poll_secs: Option<u32>,
}

/// Check a sealed poll, hold it while nothing waits, and say what to answer
/// with. `now` is a function: a held poll answers later than it arrived.
pub async fn answer(
    storage: &dyn Storage,
    waiters: &CommandWaiters,
    hub_secrets: &[&StaticSecret],
    sealed: &[u8],
    now: impl Fn() -> DateTime<Utc>,
) -> Result<(CommandPoll, Answer), PollRefusal> {
    let poll = common::poll::open_poll(sealed, hub_secrets, now())?;
    let record = storage
        .probe_record(&poll.probe_id)
        .await
        .map_err(PollRefusal::Storage)?
        .ok_or_else(|| PollRefusal::UnknownProbe(poll.probe_id.clone()))?;
    if !record.is_active() {
        return Err(PollRefusal::Revoked(poll.probe_id.clone()));
    }
    let waiting = commands::to_send(storage, &record, now())
        .await
        .map_err(PollRefusal::Storage)?;
    if !waiting.is_empty() {
        return Ok((poll, answer_now(waiting)));
    }
    match waiters.hold(&poll.probe_id).await {
        Held::Full => Ok((
            poll,
            Answer {
                commands: Vec::new(),
                next_poll_secs: Some(FALLBACK_POLL_SECS),
            },
        )),
        Held::Woken | Held::TimedOut => {
            let waiting = commands::to_send(storage, &record, now())
                .await
                .map_err(PollRefusal::Storage)?;
            Ok((poll, answer_now(waiting)))
        }
    }
}

fn answer_now(commands: Vec<Command>) -> Answer {
    Answer {
        commands,
        next_poll_secs: Some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_hold_ends_when_woken_or_after_its_time() {
        let waiters = Arc::new(CommandWaiters::new(Duration::from_millis(200)));
        assert_eq!(waiters.hold("p1").await, Held::TimedOut);
        assert_eq!(waiters.held(), 0);

        let held = tokio::spawn({
            let waiters = waiters.clone();
            async move { waiters.hold("p1").await }
        });
        while waiters.held() == 0 {
            tokio::task::yield_now().await;
        }
        waiters.wake("p2"); // another probe: nothing
        waiters.wake("p1");
        assert_eq!(held.await.unwrap(), Held::Woken);
        assert_eq!(waiters.held(), 0);
    }

    #[tokio::test]
    async fn a_new_poll_releases_the_one_before() {
        let waiters = Arc::new(CommandWaiters::new(Duration::from_secs(30)));
        let first = tokio::spawn({
            let waiters = waiters.clone();
            async move { waiters.hold("p1").await }
        });
        while waiters.held() == 0 {
            tokio::task::yield_now().await;
        }
        let second = tokio::spawn({
            let waiters = waiters.clone();
            async move { waiters.hold("p1").await }
        });
        assert_eq!(first.await.unwrap(), Held::Woken);
        // The second is the one held now, and wakes for a command.
        assert_eq!(waiters.held(), 1);
        waiters.wake("p1");
        assert_eq!(second.await.unwrap(), Held::Woken);
        assert_eq!(waiters.held(), 0);
    }
}
