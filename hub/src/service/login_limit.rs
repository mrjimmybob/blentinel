//! Slowing down password guessing (D-43).
//!
//! In memory (a restart forgets, which is fine: locks are short). Two rules:
//!
//! - **Per (client IP, username):** after [`FREE_FAILURES`] failures, each
//!   further one locks that pair for [`FIRST_LOCK`], doubling up to [`MAX_LOCK`].
//!   A successful login clears it.
//! - **Per client IP:** [`IP_FAILURES`] failures within [`IP_WINDOW`], across
//!   any usernames, lock the IP for [`MAX_LOCK`] (password spraying).
//!
//! A locked attempt is refused before the password is even checked. Wrong
//! setup codes count under the username `#setup`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Duration, Utc};

pub const FREE_FAILURES: u32 = 5;
pub const FIRST_LOCK: Duration = Duration::seconds(30);
pub const MAX_LOCK: Duration = Duration::minutes(15);
pub const IP_FAILURES: u32 = 20;
pub const IP_WINDOW: Duration = Duration::hours(1);

/// The username under which wrong setup codes are counted.
pub const SETUP: &str = "#setup";

#[derive(Debug, Default)]
struct Record {
    failures: u32,
    last_failure: Option<DateTime<Utc>>,
    locked_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct IpRecord {
    window_start: Option<DateTime<Utc>>,
    failures: u32,
    locked_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct Records {
    pairs: HashMap<(Option<IpAddr>, String), Record>,
    ips: HashMap<Option<IpAddr>, IpRecord>,
}

#[derive(Debug, Default)]
pub struct LoginLimiter {
    records: Mutex<Records>,
}

impl LoginLimiter {
    fn records(&self) -> std::sync::MutexGuard<'_, Records> {
        // Every update is a few assignments: a poisoned lock is still consistent.
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `Err(wait)` while `ip` or `(ip, username)` is locked.
    pub fn check(
        &self,
        ip: Option<IpAddr>,
        username: &str,
        now: DateTime<Utc>,
    ) -> Result<(), Duration> {
        let records = self.records();
        let pair = records.pairs.get(&(ip, key(username)));
        let until = [
            pair.and_then(|r| r.locked_until),
            records.ips.get(&ip).and_then(|r| r.locked_until),
        ]
        .into_iter()
        .flatten()
        .filter(|until| *until > now)
        .max();
        match until {
            Some(until) => Err(until - now),
            None => Ok(()),
        }
    }

    /// Record a failed attempt.
    pub fn failure(&self, ip: Option<IpAddr>, username: &str, now: DateTime<Utc>) {
        let mut records = self.records();
        // Forget pairs that have been quiet for a while (bounded memory).
        records
            .pairs
            .retain(|_, r| r.last_failure.is_some_and(|t| now - t < IP_WINDOW));

        let pair = records.pairs.entry((ip, key(username))).or_default();
        pair.failures += 1;
        pair.last_failure = Some(now);
        if pair.failures > FREE_FAILURES {
            pair.locked_until = Some(now + lock_after(pair.failures));
        }

        let per_ip = records.ips.entry(ip).or_default();
        if per_ip
            .window_start
            .is_none_or(|start| now - start >= IP_WINDOW)
        {
            per_ip.window_start = Some(now);
            per_ip.failures = 0;
        }
        per_ip.failures += 1;
        if per_ip.failures >= IP_FAILURES {
            per_ip.locked_until = Some(now + MAX_LOCK);
        }
        records.ips.retain(|_, r| {
            r.window_start
                .is_some_and(|t| now - t < IP_WINDOW + MAX_LOCK)
        });
    }

    /// A successful login: the pair starts over (the IP's count stays).
    pub fn success(&self, ip: Option<IpAddr>, username: &str) {
        self.records().pairs.remove(&(ip, key(username)));
    }
}

fn key(username: &str) -> String {
    username.trim().to_lowercase()
}

/// How long the `failures`-th failure locks (only past [`FREE_FAILURES`]).
fn lock_after(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(FREE_FAILURES + 1).min(10);
    (FIRST_LOCK * 2i32.pow(doublings)).min(MAX_LOCK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    const IP: Option<IpAddr> = Some(IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1)));

    #[test]
    fn the_lock_doubles_from_30_seconds_up_to_15_minutes() {
        assert_eq!(lock_after(6), Duration::seconds(30));
        assert_eq!(lock_after(7), Duration::seconds(60));
        assert_eq!(lock_after(8), Duration::seconds(120));
        assert_eq!(lock_after(11), Duration::seconds(960).min(MAX_LOCK));
        assert_eq!(lock_after(50), MAX_LOCK);
    }

    #[test]
    fn five_free_failures_then_locked_then_unlocked_by_time() {
        let limiter = LoginLimiter::default();
        for _ in 0..FREE_FAILURES {
            assert!(limiter.check(IP, "alice", t0()).is_ok());
            limiter.failure(IP, "alice", t0());
        }
        assert!(
            limiter.check(IP, "alice", t0()).is_ok(),
            "5 failures are free"
        );
        limiter.failure(IP, "alice", t0());
        assert_eq!(
            limiter.check(IP, "ALICE ", t0()),
            Err(FIRST_LOCK),
            "same user, any case"
        );
        assert!(
            limiter.check(IP, "bob", t0()).is_ok(),
            "other users are unaffected"
        );
        let other_ip = Some(IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 2)));
        assert!(
            limiter.check(other_ip, "alice", t0()).is_ok(),
            "per client IP"
        );
        assert!(limiter.check(IP, "alice", t0() + FIRST_LOCK).is_ok());
    }

    #[test]
    fn success_clears_the_pair() {
        let limiter = LoginLimiter::default();
        for _ in 0..=FREE_FAILURES {
            limiter.failure(IP, "alice", t0());
        }
        limiter.success(IP, "alice");
        assert!(limiter.check(IP, "alice", t0()).is_ok());
    }

    #[test]
    fn many_usernames_from_one_ip_lock_the_ip() {
        let limiter = LoginLimiter::default();
        for n in 0..IP_FAILURES {
            assert!(limiter.check(IP, &format!("user{n}"), t0()).is_ok());
            limiter.failure(IP, &format!("user{n}"), t0());
        }
        assert_eq!(limiter.check(IP, "anyone", t0()), Err(MAX_LOCK));
        assert!(limiter.check(IP, "anyone", t0() + MAX_LOCK).is_ok());
    }

    #[test]
    fn the_ip_count_restarts_every_window() {
        let limiter = LoginLimiter::default();
        for n in 0..IP_FAILURES - 1 {
            limiter.failure(IP, &format!("user{n}"), t0());
        }
        limiter.failure(IP, "late", t0() + IP_WINDOW);
        assert!(limiter.check(IP, "anyone", t0() + IP_WINDOW).is_ok());
    }
}
