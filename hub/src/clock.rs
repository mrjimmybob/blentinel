#![cfg(feature = "ssr")]
//! Injectable time source.
//!
//! Anything that depends on "now" (probe expiry, silence expiry, uptime
//! windows) gets its time from a [`Clock`] held in `AppState`, so tests can
//! control time instead of sleeping. Repository functions take `now` as a
//! plain parameter and never read the system clock themselves.

use chrono::{DateTime, Duration, Utc};
use std::sync::Mutex;

/// A source of the current time.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// The real wall clock. Used in production.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that only moves when told to. Used in tests.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    pub fn set(&self, to: DateTime<Utc>) {
        *self.lock() = to;
    }

    pub fn advance(&self, by: Duration) {
        *self.lock() += by;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DateTime<Utc>> {
        // A poisoned lock only means another test thread panicked; the value is still valid.
        self.now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let start = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let clock = ManualClock::new(start);
        assert_eq!(clock.now(), start);

        clock.advance(Duration::minutes(5));
        assert_eq!(clock.now(), start + Duration::minutes(5));

        clock.set(start);
        assert_eq!(clock.now(), start);
    }
}
