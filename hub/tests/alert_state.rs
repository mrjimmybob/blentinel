//! Alert state as stored: keys, the probe-expiry state, and a check's flap
//! state (D-63). When a check's state changes is `service::alerts::flap`.
#![cfg(feature = "ssr")]

mod support;

use hub_lib::service::alerts::flap::FlapState;
use hub_lib::service::alerts::state::{
    THRESHOLD, UP_DOWN, flap_state, probe_expiry_key, save_flap_state, should_send_alert,
    threshold_key, update_alert_state,
};
use support::*;

#[tokio::test]
async fn threshold_key_format_is_stable() {
    // Persisted in alert_states: changing it would orphan existing state.
    let check = common::checks::CheckId("0123abcd".into());
    assert_eq!(threshold_key(&check), "0123abcd-threshold");
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
async fn a_flap_state_round_trips_as_the_status_people_know() {
    let hub = TestHub::new(&[]).await;
    let store = hub.store();
    assert_eq!(flap_state(store, "k", UP_DOWN).await.unwrap(), None);

    // Confirmed up, one Down so far.
    let pending = FlapState {
        failing: false,
        streak: 1,
    };
    save_flap_state(store, "k", UP_DOWN, pending, false, hub.now())
        .await
        .unwrap();
    assert_eq!(
        flap_state(store, "k", UP_DOWN).await.unwrap(),
        Some(pending)
    );
    let stored = store.alert_state("k").await.unwrap().unwrap();
    assert_eq!((stored.last_status.as_str(), stored.streak), ("Up", 1));

    let over = FlapState {
        failing: true,
        streak: 0,
    };
    save_flap_state(store, "t", THRESHOLD, over, true, hub.now())
        .await
        .unwrap();
    let stored = store.alert_state("t").await.unwrap().unwrap();
    assert_eq!(stored.last_status, "threshold_exceeded");
    assert_eq!(stored.last_alert_sent_at, Some(hub.now()));
}
