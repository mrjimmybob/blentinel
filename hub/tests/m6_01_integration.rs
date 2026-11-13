#![cfg(feature = "ssr")]
//! M6-01 integration tests: workflow verification and API contracts

mod support;

use hub_lib::model::IncidentInfo;
use support::*;

#[tokio::test]
async fn m6_03_problems_lists_open_incidents() {
    // Verify M6-03 Problems page can fetch open incidents
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    // Trigger an incident by creating a down status
    hub.evaluate_report(
        &probe,
        vec![ping("api", "1", false), ping("api", "1", false)],
    )
    .await;

    // Fetch open incidents
    let incidents: Vec<IncidentInfo> = hub.get_json("/api/incidents?state=open").await;
    assert!(
        !incidents.is_empty(),
        "Should list open incidents when checks are down"
    );
    // Status is "Down" (title case) in the API
    assert_eq!(incidents[0].status.as_str(), "Down");
}
