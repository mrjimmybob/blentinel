//! Probe ↔ hub protocol constants. Both sides import these, so the rules the
//! probe follows (what it buffers, what it sends) and the rules the hub
//! enforces (what it accepts) cannot drift apart.

use chrono::{DateTime, Duration, Utc};

/// Protocol version this build speaks.
///
/// - **1**: legacy. Reports carry no `protocol_version` field.
/// - **2**: reports carry a signed `protocol_version`; the hub enforces replay
///   protection (timestamp window + strictly increasing timestamps per probe).
/// - **3**: the hub answers a report with a signed, sealed
///   [`ReportAck`](crate::ack::ReportAck) (M2-03, D-48). Reports are unchanged,
///   so a v2 probe still reports; it just ignores the answer.
/// - **4**: checks are hub records (M2-04, D-49). Reports carry one result
///   per check, keyed by its ID, with any number of metrics, and the revision
///   of the config the probe runs; the answer carries a new config when the
///   probe runs another one.
/// - **5**: reports carry the probe's version and its last failed update;
///   the answer may offer a signed release (M2-15, D-55). The new fields are
///   appended and optional, so a v4 report is still accepted.
/// - **6**: the answer may carry commands, and reports the results of the
///   commands the probe ran (M2-06, D-56). Appended and omitted when empty,
///   so a v5 report was still accepted.
/// - **7**: reports say which hub pins the probe holds; the answer may
///   announce the hub's next pins, or that it switched to them (M2-13,
///   D-61). The TLS pin is the certificate's public key. Appended and
///   optional, so a v6 report is still accepted; rotations go only to v7
///   probes ([`ROTATION_SINCE_PROTOCOL`]).
/// - **8**: reports carry the probe's health: when it started, the reports
///   it holds back, its clock against the hub's, its last problem (M4-02,
///   D-73). Appended and optional, so a v7 report was still accepted.
/// - **9**: a report may carry the inventory of the probe's machine: its
///   system, disks, running services, interfaces and gateway (M5-12, D-85).
///   Appended and optional, so a v8 report was still accepted.
/// - **10**: the `discover` command, and a report may carry what the probe
///   found on its network (M5-18, D-101). Appended and optional, so a v9
///   report is still accepted; a v9 probe answers that it doesn't know
///   the command.
pub const PROTOCOL_VERSION: u16 = 10;

/// The first protocol version that understands pin rotation (D-61).
pub const ROTATION_SINCE_PROTOCOL: u16 = 7;

/// Oldest protocol version the hub still accepts: current and previous
/// (ENGINEERING.md §2.7): a v9 probe updates itself to v10 (D-55, D-101).
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u16 = 9;

/// HTTP header carrying the sender's protocol version (informational; the
/// authoritative value is the signed field inside the report).
pub const PROTOCOL_HEADER: &str = "X-Blentinel-Protocol";

/// How long a probe keeps an undelivered report. Older ones are discarded,
/// and the hub rejects anything older than this (plus clock skew), so the two
/// limits must stay identical.
pub const MAX_BUFFERED_REPORT_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// Tolerated difference between probe and hub clocks.
pub const MAX_CLOCK_SKEW_SECS: i64 = 5 * 60;

/// A clock further off the hub's than this is worth a warning, well before
/// the hub refuses reports for it ([`MAX_CLOCK_SKEW_SECS`]): the probe logs
/// one, the fleet view flags it (D-73).
pub const CLOCK_WARNING_SECS: i64 = 60;

/// Body of every hub error response (4xx/5xx), on the probe endpoints and the
/// web API alike: `{"error": {"code": "unknown_probe", "message": "…"}}`.
///
/// `code` is stable and meant for programs; `message` is for people and may
/// change. The codes are listed in `docs/reference/api.md`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

/// The `error` object inside an [`ErrorBody`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}

impl std::fmt::Display for ErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.error.message, self.error.code)
    }
}

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
    fn error_body_wire_format() {
        let body = ErrorBody::new("unknown_probe", "probe is not registered");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"error": {"code": "unknown_probe", "message": "probe is not registered"}})
        );
        assert_eq!(serde_json::from_value::<ErrorBody>(json).unwrap(), body);
        assert_eq!(body.to_string(), "probe is not registered (unknown_probe)");
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
