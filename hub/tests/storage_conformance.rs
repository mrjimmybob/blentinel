//! Storage conformance suite (proposal 0011 §7): the contract every storage
//! backend must meet, written once against the `Storage` traits only (no SQL).
//!
//! Each backend gets one `conformance_suite!` line. A backend counts as
//! supported only when the whole suite passes against it.
#![cfg(feature = "ssr")]

use chrono::{DateTime, Duration, Utc};
use common::models::{Health, ResourceStatus, ResourceType, StatusReport};
use hub_lib::model::{Role, Severity};
use hub_lib::storage::{NewUser, SaveOutcome, Storage, UptimeRange, UserUpdate};

// ---------------------------------------------------------------------------
// Backends under test
// ---------------------------------------------------------------------------

/// Generates one `#[tokio::test]` per contract below for a backend.
/// `$open` must evaluate to a future yielding `(Box<dyn Storage>, guard)`;
/// the guard keeps temporary resources alive for the test's duration.
macro_rules! conformance_suite {
    ($backend:ident, $open:expr) => {
        mod $backend {
            use super::*;
            conformance_suite!(@tests $open;
                reports_are_stored_only_if_newer_than_the_probes_latest,
                replay_ordering_is_per_probe,
                probe_devices_come_from_the_latest_report_sorted_by_name,
                heartbeats_expire_once_and_reactivate,
                dashboard_aggregates_per_company_with_severity,
                company_probes_are_most_recent_first_without_names,
                company_ids_and_all_probes_list_known_probes,
                uptime_buckets_by_range,
                deleting_reports_keeps_probes_and_removing_a_probe_forgets_it,
                alert_state_round_trip_and_conditional_update,
                silences_upsert_expire_and_delete,
                archives_move_old_reports_into_a_readable_archive,
                users_round_trip_with_case_insensitive_usernames,
                user_updates_replace_fields_and_companies,
                sessions_expire_when_idle_or_past_their_end,
                sessions_end_on_logout_deactivation_and_cleanup,
            );
        }
    };
    (@tests $open:expr; $($test:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $test() {
                let (storage, _guard) = $open.await;
                super::$test(storage.as_ref()).await;
            }
        )*
    };
}

async fn open_sqlite() -> (Box<dyn Storage>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let storage = hub_lib::storage::SqliteStorage::open(&dir.path().join("hub.db"))
        .await
        .expect("open SQLite storage");
    (Box::new(storage), dir)
}

conformance_suite!(sqlite, open_sqlite());

// Future backends (ROADMAP M7), e.g.:
// conformance_suite!(postgres, open_postgres_from_env()); // BLENTINEL_TEST_POSTGRES_URL

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn t(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .with_timezone(&Utc)
}

fn now() -> DateTime<Utc> {
    t("2026-03-10T12:00:00Z")
}

fn device(name: &str, up: bool) -> ResourceStatus {
    ResourceStatus {
        name: name.into(),
        resource_type: ResourceType::Ping,
        target: format!("{name}.lan"),
        status: if up { Health::Up } else { Health::Down },
        message: String::new(),
        latency_ms: up.then_some(5),
        metric_value: None,
        metric_unit: None,
    }
}

fn report(
    probe: &str,
    company: &str,
    at: DateTime<Utc>,
    devices: Vec<ResourceStatus>,
) -> StatusReport {
    StatusReport {
        probe_id: probe.into(),
        company_id: company.into(),
        hostname: format!("{probe}-host"),
        site: "main".into(),
        timestamp: at,
        interval_seconds: 60,
        resources: devices,
        signature: None,
        ephemeral_public_key: None,
        protocol_version: Some(common::protocol::PROTOCOL_VERSION),
    }
}

/// Store a report and record the heartbeat, as ingestion does.
async fn ingest(
    s: &dyn Storage,
    probe: &str,
    company: &str,
    at: DateTime<Utc>,
    devices: Vec<ResourceStatus>,
) {
    assert_eq!(
        s.save_report(&report(probe, company, at, devices))
            .await
            .unwrap(),
        SaveOutcome::Stored
    );
    s.record_heartbeat(probe, company, at).await.unwrap();
}

// ---------------------------------------------------------------------------
// Contracts
// ---------------------------------------------------------------------------

async fn reports_are_stored_only_if_newer_than_the_probes_latest(s: &dyn Storage) {
    let r = |at| report("p1", "acme", at, vec![device("a", true)]);
    assert_eq!(s.save_report(&r(now())).await.unwrap(), SaveOutcome::Stored);

    assert_eq!(
        s.save_report(&r(now())).await.unwrap(),
        SaveOutcome::NotNewer,
        "same timestamp"
    );
    assert_eq!(
        s.save_report(&r(now() - Duration::seconds(1)))
            .await
            .unwrap(),
        SaveOutcome::NotNewer,
        "older"
    );
    assert_eq!(
        s.save_report(&r(now() + Duration::milliseconds(1)))
            .await
            .unwrap(),
        SaveOutcome::Stored,
        "1 ms newer"
    );
}

async fn replay_ordering_is_per_probe(s: &dyn Storage) {
    assert_eq!(
        s.save_report(&report("p1", "acme", now(), vec![]))
            .await
            .unwrap(),
        SaveOutcome::Stored
    );
    assert_eq!(
        s.save_report(&report("p2", "acme", now(), vec![]))
            .await
            .unwrap(),
        SaveOutcome::Stored
    );
}

async fn probe_devices_come_from_the_latest_report_sorted_by_name(s: &dyn Storage) {
    ingest(s, "p1", "acme", now(), vec![device("old", true)]).await;
    ingest(
        s,
        "p1",
        "acme",
        now() + Duration::minutes(1),
        vec![device("zeta", false), device("alpha", true)],
    )
    .await;

    let devices = s.probe_devices("p1").await.unwrap();

    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta"]);
    assert_eq!(devices[0].status, "Up");
    assert_eq!(devices[0].resource_type, "ping");
    assert_eq!(devices[0].latency_ms, Some(5));
    assert_eq!(devices[1].status, "Down");
    assert!(s.probe_devices("unknown").await.unwrap().is_empty());
}

async fn heartbeats_expire_once_and_reactivate(s: &dyn Storage) {
    ingest(s, "p1", "acme", now(), vec![]).await;

    assert!(
        s.expire_silent_probes(300, now() + Duration::seconds(299))
            .await
            .unwrap()
            .is_empty()
    );
    let later = now() + Duration::seconds(301);
    assert_eq!(
        s.expire_silent_probes(300, later).await.unwrap(),
        vec![("p1".into(), "acme".into())]
    );
    assert!(
        s.expire_silent_probes(300, later).await.unwrap().is_empty(),
        "reported once"
    );
    assert_eq!(s.company_probes("acme").await.unwrap()[0].status, "expired");

    s.record_heartbeat("p1", "acme", later).await.unwrap();
    assert_eq!(s.company_probes("acme").await.unwrap()[0].status, "active");
}

async fn dashboard_aggregates_per_company_with_severity(s: &dyn Storage) {
    // acme: live probe with an outage + a probe that will expire
    ingest(
        s,
        "stale",
        "acme",
        now() - Duration::minutes(10),
        vec![device("x", true)],
    )
    .await;
    ingest(
        s,
        "live",
        "acme",
        now(),
        vec![device("a", true), device("b", false)],
    )
    .await;
    // zeta: only a stale probe whose last report had a device down
    ingest(
        s,
        "z",
        "zeta",
        now() - Duration::minutes(10),
        vec![device("z", false)],
    )
    .await;
    s.expire_silent_probes(300, now()).await.unwrap();

    let companies = s.dashboard_companies().await.unwrap();

    let ids: Vec<&str> = companies.iter().map(|c| c.company_id.as_str()).collect();
    assert_eq!(ids, ["acme", "zeta"]);
    let acme = &companies[0];
    assert_eq!(
        (acme.total_probes, acme.active_probes, acme.expired_probes),
        (2, 1, 1)
    );
    assert_eq!((acme.devices_up, acme.devices_down), (2, 1));
    assert_eq!(
        acme.severity,
        Severity::Critical,
        "a live outage wins over an expired probe"
    );
    assert_eq!(acme.last_report.as_deref(), Some("2026-03-10T12:00:00Z"));
    assert_eq!(
        companies[1].severity,
        Severity::Degraded,
        "stale down counts only degrade"
    );
}

async fn company_probes_are_most_recent_first_without_names(s: &dyn Storage) {
    ingest(s, "older", "acme", now(), vec![device("a", false)]).await;
    ingest(
        s,
        "newer",
        "acme",
        now() + Duration::seconds(30),
        vec![device("b", true)],
    )
    .await;
    ingest(s, "other", "zeta", now(), vec![]).await;

    let probes = s.company_probes("acme").await.unwrap();

    let ids: Vec<&str> = probes.iter().map(|p| p.probe_id.as_str()).collect();
    assert_eq!(ids, ["newer", "older"]);
    assert!(
        probes.iter().all(|p| p.probe_name.is_empty()),
        "names come from config, not storage"
    );
    assert_eq!(probes[0].hostname.as_deref(), Some("newer-host"));
    assert_eq!(
        probes[0].last_seen_at.as_deref(),
        Some("2026-03-10T12:00:30Z")
    );
    assert_eq!(
        (probes[1].devices_down, probes[1].severity),
        (1, Severity::Critical)
    );
    assert!(s.company_probes("nobody").await.unwrap().is_empty());
}

async fn company_ids_and_all_probes_list_known_probes(s: &dyn Storage) {
    ingest(s, "p2", "zeta", now(), vec![]).await;
    ingest(s, "p1", "acme", now(), vec![]).await;

    assert_eq!(s.company_ids().await.unwrap(), ["acme", "zeta"]);
    let probes = s.all_probes().await.unwrap();
    let pairs: Vec<(&str, &str)> = probes
        .iter()
        .map(|p| (p.company_id.as_str(), p.probe_id.as_str()))
        .collect();
    assert_eq!(pairs, [("acme", "p1"), ("zeta", "p2")]);
    assert_eq!(
        probes[0].first_seen_at.as_deref(),
        Some("2026-03-10T12:00:00Z")
    );
}

async fn uptime_buckets_by_range(s: &dyn Storage) {
    for (at, up) in [
        ("2026-02-07T09:00:00Z", true), // 31 days ago: only in "all"
        ("2026-03-10T10:03:00Z", true),
        ("2026-03-10T10:07:00Z", false),
        ("2026-03-10T10:20:00Z", true),
    ] {
        s.save_report(&report("p1", "acme", t(at), vec![device("a", up)]))
            .await
            .unwrap();
    }
    s.save_report(&report(
        "p2",
        "zeta",
        t("2026-03-10T10:05:00Z"),
        vec![device("z", false)],
    ))
    .await
    .unwrap();

    let buckets = |range| async move {
        s.company_uptime("acme", range, now())
            .await
            .unwrap()
            .into_iter()
            .map(|b| (b.bucket, b.up_count, b.down_count))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        buckets(UptimeRange::Hours24).await,
        [
            ("2026-03-10 10:00".to_string(), 1, 1),
            ("2026-03-10 10:15".to_string(), 1, 0)
        ]
    );
    assert_eq!(
        buckets(UptimeRange::Days7).await,
        [("2026-03-10 10:00".to_string(), 2, 1)]
    );
    assert_eq!(
        buckets(UptimeRange::Days30).await,
        [("2026-03-10 08:00".to_string(), 2, 1)]
    );
    assert_eq!(
        buckets(UptimeRange::All).await,
        [
            ("2026-02-07 00:00".to_string(), 1, 0),
            ("2026-03-10 00:00".to_string(), 2, 1)
        ]
    );
}

async fn deleting_reports_keeps_probes_and_removing_a_probe_forgets_it(s: &dyn Storage) {
    ingest(s, "p1", "acme", now(), vec![device("a", true)]).await;
    ingest(s, "p2", "acme", now(), vec![device("b", true)]).await;
    ingest(s, "p3", "zeta", now(), vec![device("c", true)]).await;

    s.delete_company_reports("acme").await.unwrap();
    assert!(s.probe_devices("p1").await.unwrap().is_empty());
    assert_eq!(
        s.company_probes("acme").await.unwrap().len(),
        2,
        "probes stay known"
    );
    assert_eq!(
        s.probe_devices("p3").await.unwrap().len(),
        1,
        "other companies untouched"
    );

    s.delete_probe_reports("p3").await.unwrap();
    assert!(s.probe_devices("p3").await.unwrap().is_empty());
    assert_eq!(s.company_probes("zeta").await.unwrap().len(), 1);

    s.remove_probe("p3").await.unwrap();
    assert_eq!(s.company_ids().await.unwrap(), ["acme"]);
}

async fn alert_state_round_trip_and_conditional_update(s: &dyn Storage) {
    assert!(s.alert_state("k").await.unwrap().is_none());
    s.set_alert_status_if_present("k", "active").await.unwrap();
    assert!(
        s.alert_state("k").await.unwrap().is_none(),
        "no row is created"
    );

    s.upsert_alert_state("k", "Down", true, now())
        .await
        .unwrap();
    let state = s.alert_state("k").await.unwrap().unwrap();
    assert_eq!(
        (state.last_status.as_str(), state.last_alert_sent_at),
        ("Down", Some(now()))
    );

    s.upsert_alert_state("k", "Up", false, now() + Duration::minutes(1))
        .await
        .unwrap();
    let state = s.alert_state("k").await.unwrap().unwrap();
    assert_eq!(state.last_status, "Up");
    assert_eq!(
        state.last_alert_sent_at,
        Some(now()),
        "not sending keeps the last send time"
    );

    s.set_alert_status_if_present("k", "expired").await.unwrap();
    assert_eq!(
        s.alert_state("k").await.unwrap().unwrap().last_status,
        "expired"
    );
}

async fn silences_upsert_expire_and_delete(s: &dyn Storage) {
    let first = s
        .create_silence(
            "resource",
            "k1",
            "first",
            Some(now() + Duration::hours(1)),
            now(),
        )
        .await
        .unwrap();
    let again = s
        .create_silence("resource", "k1", "second", None, now())
        .await
        .unwrap();
    assert_eq!(first, again, "same scope updates the existing silence");
    let timed = s
        .create_silence(
            "resource",
            "k2",
            "reboot",
            Some(now() + Duration::hours(1)),
            now(),
        )
        .await
        .unwrap();

    let active = s.active_silences(now()).await.unwrap();
    assert_eq!(active.len(), 2);
    assert!(
        active
            .iter()
            .any(|x| x.scope_id == "k1" && x.reason == "second" && x.expires_at.is_none())
    );

    let later = now() + Duration::hours(2);
    assert!(
        !s.is_silenced("resource", "k2", later).await.unwrap(),
        "timed silence expired"
    );
    assert!(
        s.is_silenced("resource", "k1", later).await.unwrap(),
        "forever silence remains"
    );
    assert_eq!(s.active_silences(later).await.unwrap().len(), 1);

    s.delete_silence(timed).await.unwrap();
    s.clear_resource_silence("k1").await.unwrap();
    assert!(s.active_silences(now()).await.unwrap().is_empty());
}

async fn archives_move_old_reports_into_a_readable_archive(s: &dyn Storage) {
    let archive_dir = tempfile::tempdir().unwrap();
    let dir = archive_dir.path().to_str().unwrap();
    assert!(s.archives().await.unwrap().is_empty());
    assert!(
        s.open_archive(dir, 1).await.unwrap().is_none(),
        "unknown archive"
    );

    ingest(
        s,
        "old",
        "acme",
        now() - Duration::days(100),
        vec![device("a", false)],
    )
    .await;
    ingest(s, "new", "acme", now(), vec![device("b", true)]).await;
    // A probe with history before the cutoff that is still reporting (D-26).
    ingest(
        s,
        "alive",
        "zeta",
        now() - Duration::days(95),
        vec![device("z", true)],
    )
    .await;
    ingest(s, "alive", "zeta", now(), vec![device("z", true)]).await;
    assert_eq!(s.latest_report_at().await.unwrap(), Some(now()));

    s.create_user(&new_user("u1", "alice", Role::Owner), now())
        .await
        .unwrap();

    let created = s
        .create_archive(dir, now() - Duration::days(90), now())
        .await
        .unwrap();

    let archives = s.archives().await.unwrap();
    assert_eq!(archives.len(), 1);
    assert_eq!(archives[0].filename, created.filename);
    assert!(
        s.probe_devices("old").await.unwrap().is_empty(),
        "old data left the main store"
    );
    assert_eq!(
        s.probe_devices("new").await.unwrap().len(),
        1,
        "recent data stays"
    );

    let archive = s
        .open_archive(dir, archives[0].id)
        .await
        .unwrap()
        .expect("archive opens");
    assert_eq!(
        archive.probe_devices("old").await.unwrap().len(),
        1,
        "old data is in the archive"
    );
    assert!(archive.probe_devices("new").await.unwrap().is_empty());
    assert_eq!(
        archive.latest_report_at().await.unwrap(),
        Some(now() - Duration::days(95)),
        "the archive's data ends before the cutoff"
    );

    // Probes stay known to the archive iff it holds reports from them, and are
    // shown as last seen at their last archived report.
    let known: Vec<String> = archive
        .all_probes()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.probe_id)
        .collect();
    assert_eq!(known, ["old", "alive"]);
    let alive = archive.company_probes("zeta").await.unwrap();
    assert_eq!(alive.len(), 1);
    let last_seen: DateTime<Utc> = alive[0].last_seen_at.as_deref().unwrap().parse().unwrap();
    assert_eq!(last_seen, now() - Duration::days(95));
    assert_eq!(
        s.probe_devices("alive").await.unwrap().len(),
        1,
        "its recent data stays in the main store"
    );
    assert_eq!(
        archive.user_count().await.unwrap(),
        0,
        "archives never contain accounts"
    );
    assert_eq!(s.user_count().await.unwrap(), 1, "the hub keeps its users");
    assert!(s.size_mb().await.is_ok());
}

fn new_user(id: &str, username: &str, role: Role) -> NewUser {
    NewUser {
        id: id.into(),
        username: username.into(),
        display_name: format!("{username} display"),
        email: None,
        password_hash: format!("hash-of-{username}"),
        role,
        all_companies: true,
        companies: vec![],
        must_change_password: false,
    }
}

async fn users_round_trip_with_case_insensitive_usernames(s: &dyn Storage) {
    assert_eq!(s.user_count().await.unwrap(), 0);
    s.create_user(&new_user("u1", "Alice", Role::Owner), now())
        .await
        .unwrap();
    s.create_user(&new_user("u2", "bob", Role::Viewer), now())
        .await
        .unwrap();
    assert!(
        s.create_user(&new_user("u3", "ALICE", Role::Viewer), now())
            .await
            .is_err(),
        "usernames are unique regardless of case"
    );

    let alice = s.user_by_username("alice").await.unwrap().expect("found");
    assert_eq!(alice.info.id, "u1");
    assert_eq!(alice.info.username, "Alice");
    assert_eq!(alice.info.role, Role::Owner);
    assert_eq!(alice.password_hash, "hash-of-Alice");
    assert!(alice.info.is_active && alice.info.all_companies);
    assert_eq!(alice.info.last_login_at, None);
    assert_eq!(s.user("u2").await.unwrap().unwrap().info.username, "bob");
    assert!(s.user("nobody").await.unwrap().is_none());

    s.record_login("u1", now()).await.unwrap();
    s.set_password("u1", "new-hash", true).await.unwrap();
    let alice = s.user("u1").await.unwrap().unwrap();
    assert_eq!(alice.password_hash, "new-hash");
    assert!(alice.info.must_change_password);
    assert_eq!(
        alice.info.last_login_at.as_deref(),
        Some("2026-03-10T12:00:00Z")
    );

    let names: Vec<String> = s
        .users()
        .await
        .unwrap()
        .into_iter()
        .map(|u| u.username)
        .collect();
    assert_eq!(names, ["Alice", "bob"]);
    assert_eq!(s.user_count().await.unwrap(), 2);
    assert_eq!(s.active_owner_count().await.unwrap(), 1);
}

async fn user_updates_replace_fields_and_companies(s: &dyn Storage) {
    s.create_user(&new_user("u1", "carol", Role::Viewer), now())
        .await
        .unwrap();
    let update = |companies: Vec<&str>, is_active| UserUpdate {
        display_name: "Carol C".into(),
        email: Some("carol@example.com".into()),
        role: Role::Technician,
        all_companies: false,
        companies: companies.into_iter().map(String::from).collect(),
        is_active,
    };
    s.update_user("u1", &update(vec!["zeta", "acme"], true))
        .await
        .unwrap();
    let carol = s.user("u1").await.unwrap().unwrap().info;
    assert_eq!(carol.display_name, "Carol C");
    assert_eq!(carol.email.as_deref(), Some("carol@example.com"));
    assert_eq!(carol.role, Role::Technician);
    assert!(!carol.all_companies);
    assert_eq!(carol.companies, ["acme", "zeta"]);
    assert_eq!(s.users().await.unwrap()[0].companies, ["acme", "zeta"]);

    s.update_user("u1", &update(vec!["acme"], false))
        .await
        .unwrap();
    let carol = s.user("u1").await.unwrap().unwrap().info;
    assert_eq!(carol.companies, ["acme"]);
    assert!(!carol.is_active);
}

async fn sessions_expire_when_idle_or_past_their_end(s: &dyn Storage) {
    let idle = Duration::hours(12);
    s.create_user(&new_user("u1", "dave", Role::Admin), now())
        .await
        .unwrap();
    s.create_session("s1", "u1", now(), now() + Duration::days(7))
        .await
        .unwrap();

    let resumed = s.resume_session("s1", now() + Duration::hours(11), idle);
    assert_eq!(resumed.await.unwrap().expect("live").info.id, "u1");
    // Resuming slides the idle window: 11 h after the last use is still fine.
    let resumed = s.resume_session("s1", now() + Duration::hours(22), idle);
    assert!(resumed.await.unwrap().is_some(), "idle expiry slides");
    let resumed = s.resume_session("s1", now() + Duration::hours(35), idle);
    assert!(resumed.await.unwrap().is_none(), "idle for 13 h");

    s.create_session("s2", "u1", now(), now() + Duration::days(7))
        .await
        .unwrap();
    // Used every 10 hours (never idle) until just before the 7-day end.
    for hours in (10..168).step_by(10) {
        let at = now() + Duration::hours(hours);
        assert!(s.resume_session("s2", at, idle).await.unwrap().is_some());
    }
    let at = now() + Duration::days(7);
    assert!(
        s.resume_session("s2", at, idle).await.unwrap().is_none(),
        "absolute expiry, however active"
    );
    assert!(
        s.resume_session("unknown", now(), idle)
            .await
            .unwrap()
            .is_none()
    );
}

async fn sessions_end_on_logout_deactivation_and_cleanup(s: &dyn Storage) {
    let idle = Duration::hours(12);
    let end = now() + Duration::days(7);
    s.create_user(&new_user("u1", "erin", Role::Admin), now())
        .await
        .unwrap();
    for id in ["a", "b", "c"] {
        s.create_session(id, "u1", now(), end).await.unwrap();
    }
    let live = async |id: &str| s.resume_session(id, now(), idle).await.unwrap().is_some();

    s.delete_session("a").await.unwrap();
    assert!(!live("a").await && live("b").await);

    s.delete_user_sessions("u1", Some("b")).await.unwrap();
    assert!(live("b").await && !live("c").await, "all but the kept one");

    let deactivate = UserUpdate {
        display_name: "erin".into(),
        email: None,
        role: Role::Admin,
        all_companies: true,
        companies: vec![],
        is_active: false,
    };
    s.update_user("u1", &deactivate).await.unwrap();
    assert!(
        !live("b").await,
        "a deactivated user's sessions don't resume"
    );

    let (created, ended) = (now() - Duration::days(8), now() - Duration::days(1));
    s.create_session("old", "u1", created, ended).await.unwrap();
    assert!(s.delete_dead_sessions(now(), idle).await.unwrap() >= 1);
    s.delete_user_sessions("u1", None).await.unwrap();
    assert_eq!(s.delete_dead_sessions(now(), idle).await.unwrap(), 0);
}
