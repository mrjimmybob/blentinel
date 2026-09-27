//! Probe ↔ hub protocol constants. Both sides import these, so the rules the
//! probe follows (what it buffers, what it sends) and the rules the hub
//! enforces (what it accepts) cannot drift apart.

use chrono::{DateTime, Duration, Utc};

/// Protocol version this build speaks.
///
/// - **1**: legacy. Reports carry no `protocol_version` field.
/// - **2**: reports carry a signed `protocol_version`; the hub enforces replay
///   protection (timestamp window + strictly increasing timestamps per probe).
pub const PROTOCOL_VERSION: u16 = 2;

/// Oldest protocol version the hub still accepts (current and previous; see
/// ENGINEERING.md §2.7).
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u16 = 1;

/// HTTP header carrying the sender's protocol version (informational; the
/// authoritative value is the signed field inside the report).
pub const PROTOCOL_HEADER: &str = "X-Blentinel-Protocol";

/// How long a probe keeps an undelivered report. Older ones are discarded,
/// and the hub rejects anything older than this (plus clock skew), so the two
/// limits must stay identical.
pub const MAX_BUFFERED_REPORT_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// Tolerated difference between probe and hub clocks.
pub const MAX_CLOCK_SKEW_SECS: i64 = 5 * 60;

/// Why a report's timestamp falls outside the window the hub accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampRejection {
    /// Further in the future than [`MAX_CLOCK_SKEW_SECS`]: the probe's clock is ahead.
    InFuture,
    /// Older than [`MAX_BUFFERED_REPORT_AGE_SECS`] + skew: no probe keeps reports
    /// that long, so it is a replay of captured traffic.
    TooOld,
}

/// Check a report timestamp against the hub's clock (replay protection, part 1;
/// part 2 is "strictly newer than the probe's last stored report").
pub fn check_report_timestamp(
    timestamp: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), TimestampRejection> {
    let skew = Duration::seconds(MAX_CLOCK_SKEW_SECS);
    if timestamp > now + skew {
        return Err(TimestampRejection::InFuture);
    }
    if timestamp < now - Duration::seconds(MAX_BUFFERED_REPORT_AGE_SECS) - skew {
        return Err(TimestampRejection::TooOld);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn accepts_timestamps_within_skew_and_buffer_age() {
        let n = now();
        for ts in [
            n,
            n + Duration::seconds(MAX_CLOCK_SKEW_SECS),
            n - Duration::seconds(MAX_BUFFERED_REPORT_AGE_SECS + MAX_CLOCK_SKEW_SECS),
        ] {
            assert_eq!(check_report_timestamp(ts, n), Ok(()), "{ts}");
        }
    }

    #[test]
    fn rejects_timestamps_beyond_the_skew() {
        let n = now();
        let ts = n + Duration::seconds(MAX_CLOCK_SKEW_SECS + 1);
        assert_eq!(
            check_report_timestamp(ts, n),
            Err(TimestampRejection::InFuture)
        );
    }

    #[test]
    fn rejects_timestamps_older_than_any_probe_buffers() {
        let n = now();
        let ts = n - Duration::seconds(MAX_BUFFERED_REPORT_AGE_SECS + MAX_CLOCK_SKEW_SECS + 1);
        assert_eq!(
            check_report_timestamp(ts, n),
            Err(TimestampRejection::TooOld)
        );
    }
}
