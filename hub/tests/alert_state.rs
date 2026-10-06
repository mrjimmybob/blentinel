//! Alert state as stored: keys, the probe-expiry state, and a check's state
//! (its confirmed status, Warning included, and its streak: D-63, D-64).
//! When a check's state changes is `service::alerts::flap`.
#![cfg(feature = "ssr")]

mod support;

use hub_lib::model::Status;
use hub_lib::service::alerts::flap::FlapState;
use hub_lib::service::alerts::state::{
    flap_state, probe_expiry_key, save_flap_state, should_send_alert, update_alert_state,
};
use support::*;

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
async fn a_checks_state_round_trips_as_the_status_people_know() {
    let hub = TestHub::new(&[]).await;
    let store = hub.store();
    assert_eq!(flap_state(store, "k").await.unwrap(), None);

    // Confirmed up, one bad result so far.
    let pending = FlapState {
        confirmed: Status::Up,
        streak: 1,
    };
    save_flap_state(store, "k", pending, false, hub.now())
        .await
        .unwrap();
    assert_eq!(flap_state(store, "k").await.unwrap(), Some(pending));
    let stored = store.alert_state("k").await.unwrap().unwrap();
    assert_eq!((stored.last_status.as_str(), stored.streak), ("Up", 1));

    let warning = FlapState {
        confirmed: Status::Warning,
        streak: 0,
    };
    save_flap_state(store, "k", warning, true, hub.now())
        .await
        .unwrap();
    let stored = store.alert_state("k").await.unwrap().unwrap();
    assert_eq!(stored.last_status, "Warning");
    assert_eq!(stored.last_alert_sent_at, Some(hub.now()));
    assert_eq!(flap_state(store, "k").await.unwrap(), Some(warning));
}
