//! Alert state transitions (current behaviour: alert on every status change,
//! including the very first Down; no flap suppression, see D-16 / proposal 0005).
#![cfg(feature = "ssr")]

mod support;

use hub_lib::service::alerts::state::{
    probe_expiry_key, resource_key, should_send_alert, update_alert_state,
};
use support::*;

#[tokio::test]
async fn resource_key_joins_company_probe_name_and_target() {
    assert_eq!(
        resource_key("acme", "p1", "NAS", "10.0.0.2"),
        "acme:p1:NAS:10.0.0.2"
    );
}

#[tokio::test]
async fn probe_expiry_key_format_is_stable() {
    // Persisted in alert_states: changing it would orphan existing state.
    assert_eq!(probe_expiry_key("acme", "p1"), "probe_expiry:acme:p1");
}

#[tokio::test]
async fn first_sighting_alerts_only_when_not_up() {
    let hub = TestHub::new(&[]).await;

    assert!(!should_send_alert(hub.store(), "k-up", "Up").await.unwrap());
    assert!(
        should_send_alert(hub.store(), "k-down", "Down")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn alerts_on_every_change_and_never_on_repeats() {
    let hub = TestHub::new(&[]).await;
    let store = hub.store();
    let now = hub.now();

    update_alert_state(store, "k", "Up", false, now)
        .await
        .unwrap();
    assert!(!should_send_alert(store, "k", "Up").await.unwrap());
    assert!(should_send_alert(store, "k", "Down").await.unwrap());

    update_alert_state(store, "k", "Down", true, now)
        .await
        .unwrap();
    assert!(!should_send_alert(store, "k", "Down").await.unwrap());
    assert!(
        should_send_alert(store, "k", "Up").await.unwrap(),
        "recovery alerts too"
    );
}

#[tokio::test]
async fn a_single_failed_check_alerts_immediately() {
    // Characterises today's behaviour that M3-02 (flap suppression) will change:
    // one Down between two Ups produces two alerts (down + recovery).
    let hub = TestHub::new(&[]).await;
    let store = hub.store();
    let now = hub.now();
    let mut alerts = 0;

    for status in ["Up", "Down", "Up"] {
        let send = should_send_alert(store, "k", status).await.unwrap();
        alerts += send as usize;
        update_alert_state(store, "k", status, send, now)
            .await
            .unwrap();
    }

    assert_eq!(alerts, 2);
}
