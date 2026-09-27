//! Company uptime history: range windows and bucket sizes
//! (24h → 15 min, 7d → 1 h, 30d → 4 h, all → 1 day). Buckets are UTC.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
use support::*;

/// Hub clock is `default_start()` = 2026-03-10T12:00:00Z for every test here.
async fn hub_with_history() -> (TestHub, TestProbe) {
    let probe = TestProbe::new("hq");
    let other = TestProbe::new("other");
    let hub = TestHub::new(&[&probe, &other]).await;

    let send = |timestamp: &str, resources| {
        let report = probe.signed_report("acme", at(timestamp), resources);
        let hub = &hub;
        async move { assert_eq!(hub.send_report(&report).await, StatusCode::OK) }
    };
    send("2026-02-07T09:00:00Z", vec![ping("a", "1", true)]).await; // 31 days ago
    send("2026-02-18T09:00:00Z", vec![ping("a", "1", false)]).await; // 20 days ago
    send("2026-03-09T11:59:00Z", vec![ping("a", "1", true)]).await; // just over 24 h ago
    send("2026-03-09T12:30:00Z", vec![ping("a", "1", true)]).await; // just under 24 h ago
    send(
        "2026-03-10T10:03:00Z",
        vec![ping("a", "1", true), ping("b", "2", false)],
    )
    .await;
    send("2026-03-10T10:07:00Z", vec![ping("a", "1", true)]).await;
    send("2026-03-10T10:20:00Z", vec![ping("a", "1", false)]).await;

    // Another company's data must never leak into acme's history.
    let noise = other.signed_report(
        "zeta",
        at("2026-03-10T10:05:00Z"),
        vec![ping("z", "9", false)],
    );
    assert_eq!(hub.send_report(&noise).await, StatusCode::OK);

    (hub, probe)
}

fn bucket(bucket: &str, up: i64, down: i64) -> Value {
    json!({ "bucket": bucket, "up_count": up, "down_count": down })
}

#[tokio::test]
async fn last_24h_uses_15_minute_buckets() {
    let (hub, _) = hub_with_history().await;

    let buckets: Value = hub.get_json("/api/company/acme/uptime?range=24h").await;

    assert_eq!(
        buckets,
        json!([
            bucket("2026-03-09 12:30", 1, 0),
            bucket("2026-03-10 10:00", 2, 1),
            bucket("2026-03-10 10:15", 0, 1),
        ])
    );
}

#[tokio::test]
async fn missing_or_unknown_range_falls_back_to_24h() {
    let (hub, _) = hub_with_history().await;
    let expected: Value = hub.get_json("/api/company/acme/uptime?range=24h").await;

    let missing: Value = hub.get_json("/api/company/acme/uptime").await;
    let unknown: Value = hub
        .get_json("/api/company/acme/uptime?range=fortnight")
        .await;

    assert_eq!(missing, expected);
    assert_eq!(unknown, expected);
}

#[tokio::test]
async fn last_7d_uses_hourly_buckets() {
    let (hub, _) = hub_with_history().await;

    let buckets: Value = hub.get_json("/api/company/acme/uptime?range=7d").await;

    assert_eq!(
        buckets,
        json!([
            bucket("2026-03-09 11:00", 1, 0),
            bucket("2026-03-09 12:00", 1, 0),
            bucket("2026-03-10 10:00", 2, 2),
        ])
    );
}

#[tokio::test]
async fn last_30d_uses_4_hour_buckets() {
    let (hub, _) = hub_with_history().await;

    let buckets: Value = hub.get_json("/api/company/acme/uptime?range=30d").await;

    assert_eq!(
        buckets,
        json!([
            bucket("2026-02-18 08:00", 0, 1),
            bucket("2026-03-09 08:00", 1, 0),
            bucket("2026-03-09 12:00", 1, 0),
            bucket("2026-03-10 08:00", 2, 2),
        ])
    );
}

#[tokio::test]
async fn all_uses_daily_buckets_over_all_retained_data() {
    let (hub, _) = hub_with_history().await;

    let buckets: Value = hub.get_json("/api/company/acme/uptime?range=all").await;

    assert_eq!(
        buckets,
        json!([
            bucket("2026-02-07 00:00", 1, 0),
            bucket("2026-02-18 00:00", 0, 1),
            bucket("2026-03-09 00:00", 2, 0),
            bucket("2026-03-10 00:00", 2, 2),
        ])
    );
}
