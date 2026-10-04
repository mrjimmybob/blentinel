//! Storage conformance suite (proposal 0011 §7): the contract every storage
//! backend must meet, written once against the `Storage` traits only (no SQL).
//!
//! Each backend gets one `conformance_suite!` line. A backend counts as
//! supported only when the whole suite passes against it.
#![cfg(feature = "ssr")]

use chrono::{DateTime, Duration, Utc};
use common::checks::{CheckId, CheckResult, CheckSpec, ConfigRejection, Health, Metric};
use common::models::{StatusReport, UpdateFailure};
use hub_lib::model::{CompanyScope, Role, SILENCE_CHECK, Severity};
use hub_lib::storage::{
    ApprovedVersion, CheckRecord, NewUser, ReportedState, SaveOutcome, Storage, UptimeRange,
    UserUpdate,
};

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
                companies_and_sites_are_created_once_and_keep_their_edits,
                archived_companies_leave_the_dashboard_but_stay_listed,
                the_dashboard_and_probes_show_display_names,
                the_audit_log_appends_and_filters_newest_first,
                probe_records_are_saved_listed_moved_and_revoked,
                enrollment_codes_are_redeemed_atomically_within_their_limits,
                checks_are_created_listed_updated_and_deleted_with_their_probe,
                a_probes_interval_and_config_state_survive_enrolling_again,
                a_probes_version_and_failed_update_follow_its_reports,
                one_release_is_approved_or_none,
                silences_of_checks_carry_their_company,
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

/// The result of the check with ID `id` (see [`give_checks`]).
fn device(id: &str, up: bool) -> CheckResult {
    CheckResult {
        check_id: CheckId(id.into()),
        status: if up { Health::Up } else { Health::Down },
        message: String::new(),
        latency_ms: up.then_some(5),
        metrics: vec![],
    }
}

fn report(
    probe: &str,
    company: &str,
    at: DateTime<Utc>,
    devices: Vec<CheckResult>,
) -> StatusReport {
    StatusReport {
        protocol_version: common::protocol::PROTOCOL_VERSION,
        probe_id: probe.into(),
        company_id: company.into(),
        hostname: format!("{probe}-host"),
        site: "main".into(),
        timestamp: at,
        interval_seconds: 60,
        results: devices,
        config_revision: None,
        config_rejected: None,
        probe_version: None,
        update_failed: None,
        signature: None,
    }
}

/// A check of `probe` with ID and name `id`, pinging `<id>.lan`.
fn check(id: &str, probe: &str) -> CheckRecord {
    CheckRecord {
        id: CheckId(id.into()),
        probe_id: probe.into(),
        company_id: String::new(), // read from the probe record
        name: id.into(),
        spec: CheckSpec::Ping {
            host: format!("{id}.lan"),
        },
        enabled: true,
        created_at: now(),
        updated_at: now(),
        updated_by: Some("alice".into()),
    }
}

/// Enroll `probe` in `company` (site `main`) and give it checks with these IDs.
async fn give_checks(s: &dyn Storage, probe: &str, company: &str, ids: &[&str]) {
    s.create_company(company, company, now()).await.unwrap();
    s.create_site(company, "main", "main", now()).await.unwrap();
    s.save_probe_record(&record(probe, company, "main"))
        .await
        .unwrap();
    for id in ids {
        s.create_check(&check(id, probe)).await.unwrap();
    }
}

/// Store a report and record the heartbeat, as ingestion does.
async fn ingest(
    s: &dyn Storage,
    probe: &str,
    company: &str,
    at: DateTime<Utc>,
    devices: Vec<CheckResult>,
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
    give_checks(s, "p1", "acme", &["old", "zeta", "alpha", "gone"]).await;
    ingest(s, "p1", "acme", now(), vec![device("old", true)]).await;
    let mut alpha = device("alpha", true);
    alpha.metrics = vec![
        Metric::new("usage", 42.5, "%"),
        Metric::new("free", 12.0, "GB"),
    ];
    ingest(
        s,
        "p1",
        "acme",
        now() + Duration::minutes(1),
        vec![device("zeta", false), alpha, device("gone", true)],
    )
    .await;
    s.delete_check(&CheckId("gone".into())).await.unwrap();

    let devices = s.probe_devices("p1").await.unwrap();

    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta"], "a deleted check is left out");
    assert_eq!(devices[0].check_id, "alpha");
    assert_eq!(devices[0].status, "Up");
    assert_eq!(devices[0].kind, "ping");
    assert_eq!(devices[0].target, "alpha.lan");
    assert_eq!(devices[0].latency_ms, Some(5));
    assert_eq!(
        devices[0].metrics,
        [
            Metric::new("usage", 42.5, "%"),
            Metric::new("free", 12.0, "GB")
        ],
        "metrics keep their order"
    );
    assert_eq!(devices[1].status, "Down");
    assert!(devices[1].metrics.is_empty());
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

    let companies = s.dashboard_companies(&CompanyScope::All).await.unwrap();

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
    give_checks(s, "p1", "acme", &["a"]).await;
    give_checks(s, "p3", "zeta", &["c"]).await;
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
            SILENCE_CHECK,
            "k1",
            "first",
            Some(now() + Duration::hours(1)),
            now(),
        )
        .await
        .unwrap();
    let again = s
        .create_silence(SILENCE_CHECK, "k1", "second", None, now())
        .await
        .unwrap();
    assert_eq!(first, again, "same scope updates the existing silence");
    let timed = s
        .create_silence(
            SILENCE_CHECK,
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
        !s.is_silenced(SILENCE_CHECK, "k2", later).await.unwrap(),
        "timed silence expired"
    );
    assert!(
        s.is_silenced(SILENCE_CHECK, "k1", later).await.unwrap(),
        "forever silence remains"
    );
    assert_eq!(s.active_silences(later).await.unwrap().len(), 1);

    s.delete_silence(timed).await.unwrap();
    s.clear_check_silence(&CheckId("k1".into())).await.unwrap();
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
    give_checks(s, "old", "acme", &["a"]).await;
    give_checks(s, "new", "acme", &["b"]).await;
    give_checks(s, "alive", "zeta", &["z"]).await;

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
    s.append_audit(&hub_lib::storage::NewAuditEntry {
        at: now(),
        action: "user.create".into(),
        ..Default::default()
    })
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
    let no_filter = hub_lib::storage::AuditFilter {
        limit: 10,
        ..Default::default()
    };
    assert!(
        archive.audit_entries(&no_filter).await.unwrap().is_empty(),
        "nor the audit log"
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
        scope: CompanyScope::All,
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
    assert!(alice.info.is_active);
    assert_eq!(alice.info.scope, CompanyScope::All);
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
    let ids = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
    let update = |scope, is_active| UserUpdate {
        display_name: "Carol C".into(),
        email: Some("carol@example.com".into()),
        role: Role::Technician,
        scope,
        is_active,
    };
    let only = CompanyScope::Only(ids(&["zeta", "acme"]));
    s.update_user("u1", &update(only.clone(), true))
        .await
        .unwrap();
    let carol = s.user("u1").await.unwrap().unwrap().info;
    assert_eq!(carol.display_name, "Carol C");
    assert_eq!(carol.email.as_deref(), Some("carol@example.com"));
    assert_eq!(carol.role, Role::Technician);
    assert_eq!(carol.scope, only);
    assert_eq!(s.users().await.unwrap()[0].scope, only);

    // "All except" is kept apart from "only", with its own list.
    let except = CompanyScope::AllExcept(ids(&["acme"]));
    s.update_user("u1", &update(except.clone(), false))
        .await
        .unwrap();
    let carol = s.user("u1").await.unwrap().unwrap().info;
    assert_eq!(carol.scope, except);
    assert!(!carol.is_active);
    s.update_user("u1", &update(CompanyScope::All, true))
        .await
        .unwrap();
    assert_eq!(
        s.user("u1").await.unwrap().unwrap().info.scope,
        CompanyScope::All
    );
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
        scope: CompanyScope::All,
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

/// A company with one site (both created by hand, D-47).
async fn place(s: &dyn Storage, company: &str, site: &str) {
    s.create_company(company, company, now()).await.unwrap();
    s.create_site(company, site, site, now()).await.unwrap();
}

async fn companies_and_sites_are_created_once_and_keep_their_edits(s: &dyn Storage) {
    use hub_lib::model::{UpdateCompanyRequest, UpdateSiteRequest};

    assert!(s.create_company("acme", "ACME", now()).await.unwrap());
    assert!(
        !s.create_company("acme", "Other", now()).await.unwrap(),
        "the ID is taken"
    );
    assert!(s.create_site("acme", "hq", "HQ", now()).await.unwrap());
    assert!(
        s.create_site("acme", "warehouse", "Warehouse", now())
            .await
            .unwrap()
    );
    assert!(!s.create_site("acme", "hq", "Again", now()).await.unwrap());
    assert!(
        !s.create_site("nobody", "hq", "HQ", now()).await.unwrap(),
        "no site without its company"
    );

    let acme = s.company("acme").await.unwrap().expect("created");
    assert_eq!(acme.company.name, "ACME", "the first name stays");
    assert!(!acme.company.is_new && !acme.company.archived);
    assert_eq!(acme.company.site_count, 2);
    let site_ids: Vec<_> = acme.sites.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(site_ids, ["hq", "warehouse"]);

    let fields = UpdateCompanyRequest {
        name: "ACME Ltd".into(),
        contact_email: Some("ops@acme.example".into()),
        ..Default::default()
    };
    assert!(s.update_company("acme", &fields, now()).await.unwrap());
    let site = UpdateSiteRequest {
        name: "Headquarters".into(),
        address: Some("1 Main St".into()),
        notes: None,
    };
    assert!(s.update_site("acme", "hq", &site, now()).await.unwrap());
    let acme = s.company("acme").await.unwrap().unwrap();
    assert_eq!(acme.company.name, "ACME Ltd");
    assert_eq!(
        acme.company.contact_email.as_deref(),
        Some("ops@acme.example")
    );
    let hq = acme.sites.iter().find(|x| x.id == "hq").unwrap();
    assert_eq!(hq.name, "Headquarters");

    assert!(!s.update_company("nobody", &fields, now()).await.unwrap());
    assert!(
        !s.update_site("acme", "nowhere", &site, now())
            .await
            .unwrap()
    );
    assert!(s.company("nobody").await.unwrap().is_none());
}

async fn archived_companies_leave_the_dashboard_but_stay_listed(s: &dyn Storage) {
    ingest(s, "p1", "acme", now(), vec![device("a", true)]).await;
    ingest(s, "p2", "zeta", now(), vec![device("z", true)]).await;
    place(s, "acme", "main").await;
    place(s, "zeta", "main").await;

    assert!(s.set_company_archived("zeta", true, now()).await.unwrap());
    let dashboard: Vec<_> = s
        .dashboard_companies(&CompanyScope::All)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.company_id)
        .collect();
    assert_eq!(dashboard, ["acme"]);
    let zeta = s.company("zeta").await.unwrap().unwrap().company;
    assert!(zeta.archived && !zeta.is_new, "archiving is a review");
    assert_eq!(zeta.probe_count, 1);
    assert_eq!(
        s.companies(&CompanyScope::All).await.unwrap().len(),
        2,
        "still listed"
    );

    assert!(s.set_company_archived("zeta", false, now()).await.unwrap());
    assert_eq!(
        s.dashboard_companies(&CompanyScope::All)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(!s.set_company_archived("nobody", true, now()).await.unwrap());
}

async fn the_dashboard_and_probes_show_display_names(s: &dyn Storage) {
    use hub_lib::model::{UpdateCompanyRequest, UpdateSiteRequest};

    ingest(s, "p1", "acme", now(), vec![device("a", true)]).await; // site "main"
    let dashboard = s.dashboard_companies(&CompanyScope::All).await.unwrap();
    assert_eq!(dashboard[0].name, "acme", "no record yet: the ID");

    place(s, "acme", "main").await;
    let fields = UpdateCompanyRequest {
        name: "ACME Ltd".into(),
        ..Default::default()
    };
    s.update_company("acme", &fields, now()).await.unwrap();
    let site = UpdateSiteRequest {
        name: "Warehouse 1".into(),
        ..Default::default()
    };
    s.update_site("acme", "main", &site, now()).await.unwrap();

    assert_eq!(
        s.dashboard_companies(&CompanyScope::All).await.unwrap()[0].name,
        "ACME Ltd"
    );
    let probes = s.company_probes("acme").await.unwrap();
    assert_eq!(probes[0].site.as_deref(), Some("Warehouse 1"));
}

async fn the_audit_log_appends_and_filters_newest_first(s: &dyn Storage) {
    use hub_lib::storage::{AuditFilter, NewAuditEntry};
    let entry = |minutes: i64, username: Option<&str>, action: &str, target: &str| NewAuditEntry {
        at: now() + Duration::minutes(minutes),
        user_id: username.map(|u| format!("id-{u}")),
        username: username.map(str::to_string),
        actor_ip: Some("192.0.2.1".into()),
        action: action.into(),
        target_type: Some("company".into()),
        target_id: Some(target.into()),
        company_id: Some(target.into()),
        details: Some(r#"{"k":1}"#.into()),
    };
    s.append_audit(&entry(0, Some("alice"), "company.update", "acme"))
        .await
        .unwrap();
    s.append_audit(&entry(1, Some("bob"), "user.create", "u1"))
        .await
        .unwrap();
    s.append_audit(&entry(2, None, "company.register", "zeta"))
        .await
        .unwrap();
    s.append_audit(&entry(3, Some("Alice"), "company.archive", "acme"))
        .await
        .unwrap();

    let all = AuditFilter {
        limit: 100,
        ..Default::default()
    };
    let got = s.audit_entries(&all).await.unwrap();
    let actions: Vec<_> = got.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        [
            "company.archive",
            "company.register",
            "user.create",
            "company.update"
        ]
    );
    assert_eq!(got[1].username, None, "the hub itself");
    assert_eq!(got[0].details.as_deref(), Some(r#"{"k":1}"#));
    assert_eq!(got[0].at, "2026-03-10T12:03:00Z");

    let by = |f: AuditFilter| async move {
        s.audit_entries(&f)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.action)
            .collect::<Vec<_>>()
    };
    let user = by(AuditFilter {
        username: Some("ALICE".into()),
        ..all.clone()
    })
    .await;
    assert_eq!(user, ["company.archive", "company.update"], "any case");
    let prefix = by(AuditFilter {
        action_prefix: Some("company".into()),
        ..all.clone()
    })
    .await;
    assert_eq!(prefix.len(), 3);
    let exact = by(AuditFilter {
        action_prefix: Some("user.create".into()),
        ..all.clone()
    })
    .await;
    assert_eq!(exact, ["user.create"]);
    let not_a_prefix = by(AuditFilter {
        action_prefix: Some("comp".into()),
        ..all.clone()
    })
    .await;
    assert!(not_a_prefix.is_empty(), "whole segments only");
    let target = by(AuditFilter {
        target: Some("acme".into()),
        ..all.clone()
    })
    .await;
    assert_eq!(target.len(), 2);
    let window = by(AuditFilter {
        from: Some(now() + Duration::minutes(1)),
        to: Some(now() + Duration::minutes(3)),
        ..all.clone()
    })
    .await;
    assert_eq!(window, ["company.register", "user.create"]);

    let first_page = s
        .audit_entries(&AuditFilter {
            limit: 2,
            ..all.clone()
        })
        .await
        .unwrap();
    assert_eq!(first_page.len(), 2);
    let next = by(AuditFilter {
        before_id: Some(first_page[1].id),
        limit: 2,
        ..all.clone()
    })
    .await;
    assert_eq!(next, ["user.create", "company.update"]);
}

fn record(id: &str, company: &str, site: &str) -> hub_lib::storage::ProbeRecord {
    hub_lib::storage::ProbeRecord {
        id: id.into(),
        name: format!("{id}-name"),
        company_id: company.into(),
        site_id: site.into(),
        state: hub_lib::storage::PROBE_ACTIVE.into(),
        hostname: Some(format!("{id}-host")),
        os: Some("linux".into()),
        arch: Some("x86_64".into()),
        probe_version: Some("0.1.0".into()),
        enrolled_at: now(),
        token_id: None,
        interval_secs: 60,
        config_revision: None,
        config_rejected: None,
        update_failed: None,
    }
}

async fn probe_records_are_saved_listed_moved_and_revoked(s: &dyn Storage) {
    place(s, "acme", "hq").await;
    s.create_site("acme", "branch", "Branch", now())
        .await
        .unwrap();
    place(s, "zeta", "main").await;
    s.save_probe_record(&record("p1", "acme", "hq"))
        .await
        .unwrap();
    s.save_probe_record(&record("p2", "zeta", "main"))
        .await
        .unwrap();
    s.record_heartbeat("p1", "acme", now()).await.unwrap();

    let p1 = s.probe_record("p1").await.unwrap().expect("saved");
    assert!(p1.is_active());
    assert_eq!(p1.hostname.as_deref(), Some("p1-host"));
    assert!(s.probe_record("nobody").await.unwrap().is_none());

    let all = s.probe_records(&CompanyScope::All).await.unwrap();
    let ids: Vec<_> = all.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["p1", "p2"], "by company name, then probe name");
    assert_eq!(all[0].company_name, "acme");
    assert_eq!(all[0].site_name, "hq");
    assert_eq!(all[0].last_seen_at.as_deref(), Some("2026-03-10T12:00:00Z"));
    assert_eq!(all[1].last_seen_at, None, "never reported");
    let only_zeta = CompanyScope::Only(["zeta".to_string()].into());
    let scoped = s.probe_records(&only_zeta).await.unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].id, "p2");

    assert!(
        s.update_probe_record("p1", "Front desk", "branch", 30)
            .await
            .unwrap()
    );
    let p1 = s.probe_record("p1").await.unwrap().unwrap();
    assert_eq!(
        (p1.name.as_str(), p1.site_id.as_str(), p1.interval_secs),
        ("Front desk", "branch", 30)
    );
    assert_eq!(
        s.probe_records(&CompanyScope::All).await.unwrap()[0].interval_secs,
        30
    );
    assert!(
        !s.update_probe_record("p1", "x", "main", 60).await.unwrap(),
        "only a site of its own company"
    );
    assert!(
        !s.update_probe_record("nobody", "x", "hq", 60)
            .await
            .unwrap()
    );

    assert!(s.revoke_probe_record("p1", now()).await.unwrap());
    assert!(!s.probe_record("p1").await.unwrap().unwrap().is_active());
    // Enrolling the same key again re-activates it.
    s.save_probe_record(&record("p1", "acme", "hq"))
        .await
        .unwrap();
    assert!(s.probe_record("p1").await.unwrap().unwrap().is_active());

    s.delete_probe_record("p2").await.unwrap();
    assert!(s.probe_record("p2").await.unwrap().is_none());
}

async fn enrollment_codes_are_redeemed_atomically_within_their_limits(s: &dyn Storage) {
    use hub_lib::storage::NewEnrollmentToken;
    place(s, "acme", "hq").await;
    place(s, "zeta", "main").await;
    let token = |id: &str, company: &str, site: &str, max_uses, hours| NewEnrollmentToken {
        id: id.into(),
        secret_hash: format!("hash-{id}"),
        company_id: company.into(),
        site_id: site.into(),
        probe_name: None,
        created_by: Some("alice".into()),
        expires_at: now() + Duration::hours(hours),
        max_uses,
    };
    s.create_enrollment_token(&token("t1", "acme", "hq", Some(2), 24), now())
        .await
        .unwrap();
    s.create_enrollment_token(&token("t2", "zeta", "main", Some(1), 1), now())
        .await
        .unwrap();

    let redeem =
        async |id: &str, hash: &str, at| s.redeem_enrollment_token(id, hash, at).await.unwrap();
    assert!(redeem("t1", "wrong", now()).await.is_none(), "wrong secret");
    let first = redeem("t1", "hash-t1", now()).await.expect("first use");
    assert_eq!(
        (first.company_id.as_str(), first.site_id.as_str()),
        ("acme", "hq")
    );
    assert!(
        redeem("t1", "hash-t1", now()).await.is_some(),
        "second of two uses"
    );
    assert!(redeem("t1", "hash-t1", now()).await.is_none(), "used up");
    assert!(
        redeem("t2", "hash-t2", now() + Duration::hours(2))
            .await
            .is_none(),
        "expired"
    );
    assert!(redeem("nobody", "x", now()).await.is_none());

    s.create_enrollment_token(&token("t3", "acme", "hq", Some(5), 24), now())
        .await
        .unwrap();
    let listed = s
        .enrollment_tokens(&CompanyScope::All, now())
        .await
        .unwrap();
    let ids: Vec<_> = listed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "t1 is used up: {ids:?}");
    assert!(ids.contains(&"t2") && ids.contains(&"t3"));
    let only_acme = CompanyScope::Only(["acme".to_string()].into());
    assert_eq!(
        s.enrollment_tokens(&only_acme, now()).await.unwrap().len(),
        1
    );
    assert_eq!(
        s.enrollment_token_company("t3").await.unwrap().as_deref(),
        Some("acme")
    );

    assert!(s.revoke_enrollment_token("t3", now()).await.unwrap());
    assert!(redeem("t3", "hash-t3", now()).await.is_none(), "revoked");
    assert!(!s.revoke_enrollment_token("nobody", now()).await.unwrap());

    // No limit: any number of probes until it expires or is revoked.
    s.create_enrollment_token(&token("t4", "acme", "hq", None, 24), now())
        .await
        .unwrap();
    for _ in 0..3 {
        assert!(redeem("t4", "hash-t4", now()).await.is_some(), "no limit");
    }
    let listed = s
        .enrollment_tokens(&CompanyScope::All, now())
        .await
        .unwrap();
    let t4 = listed.iter().find(|t| t.id == "t4").expect("still listed");
    assert_eq!((t4.max_uses, t4.uses), (None, 3));
}

async fn checks_are_created_listed_updated_and_deleted_with_their_probe(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &["b-id"]).await;
    let mut named_a = check("a-id", "p1");
    named_a.name = "a check".into();
    named_a.spec = CheckSpec::Disk { path: None };
    s.create_check(&named_a).await.unwrap();

    let checks = s.probe_checks("p1").await.unwrap();
    let names: Vec<_> = checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["a check", "b-id"], "by name");
    assert_eq!(checks[0].company_id, "acme", "from the probe record");
    assert_eq!(checks[0].spec, CheckSpec::Disk { path: None });
    assert_eq!(checks[0].updated_by.as_deref(), Some("alice"));
    assert!(s.probe_checks("nobody").await.unwrap().is_empty());

    let mut changed = checks[1].clone();
    changed.name = "Router".into();
    changed.spec = CheckSpec::Tcp {
        host: "10.0.0.1".into(),
        port: 443,
    };
    changed.enabled = false;
    changed.updated_at = now() + Duration::minutes(5);
    changed.updated_by = Some("bob".into());
    assert!(s.update_check(&changed).await.unwrap());
    let stored = s.check(&changed.id).await.unwrap().expect("still there");
    assert_eq!(stored, changed);
    let mut unknown = changed.clone();
    unknown.id = CheckId("nope".into());
    assert!(!s.update_check(&unknown).await.unwrap());

    assert!(s.delete_check(&CheckId("a-id".into())).await.unwrap());
    assert!(!s.delete_check(&CheckId("a-id".into())).await.unwrap());
    assert!(s.check(&CheckId("a-id".into())).await.unwrap().is_none());

    // A probe's checks go with it.
    s.delete_probe_record("p1").await.unwrap();
    assert!(s.check(&changed.id).await.unwrap().is_none());
}

async fn a_probes_interval_and_config_state_survive_enrolling_again(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &["c1"]).await;
    let fresh = s.probe_record("p1").await.unwrap().unwrap();
    assert_eq!(fresh.interval_secs, 60, "the default");
    assert_eq!((fresh.config_revision, fresh.config_rejected), (None, None));

    let refused = ConfigRejection {
        revision: "r2".into(),
        error: "unknown kind".into(),
    };
    let state = ReportedState {
        config_revision: Some("r1"),
        config_rejected: Some(&refused),
        ..Default::default()
    };
    s.record_probe_state("p1", &state).await.unwrap();
    assert!(s.update_probe_record("p1", "p1", "main", 15).await.unwrap());

    // Enrolling the same key again replaces its placement, not what it runs.
    s.save_probe_record(&record("p1", "acme", "main"))
        .await
        .unwrap();
    let again = s.probe_record("p1").await.unwrap().unwrap();
    assert_eq!(again.interval_secs, 15);
    assert_eq!(again.config_revision.as_deref(), Some("r1"));
    assert_eq!(again.config_rejected, Some(refused));
    assert_eq!(s.probe_checks("p1").await.unwrap().len(), 1);

    let state = ReportedState {
        config_revision: Some("r2"),
        ..Default::default()
    };
    s.record_probe_state("p1", &state).await.unwrap();
    let applied = s.probe_record("p1").await.unwrap().unwrap();
    assert_eq!(applied.config_revision.as_deref(), Some("r2"));
    assert_eq!(applied.config_rejected, None);
}

async fn a_probes_version_and_failed_update_follow_its_reports(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &["c1"]).await;
    let failed = UpdateFailure {
        version: "0.3.0".into(),
        error: "rolled back".into(),
    };
    let state = ReportedState {
        probe_version: Some("0.2.0"),
        update_failed: Some(&failed),
        ..Default::default()
    };
    s.record_probe_state("p1", &state).await.unwrap();
    let record = s.probe_record("p1").await.unwrap().unwrap();
    assert_eq!(record.probe_version.as_deref(), Some("0.2.0"));
    assert_eq!(record.update_failed.as_ref(), Some(&failed));
    let listed = s.probe_records(&CompanyScope::All).await.unwrap();
    assert_eq!(listed[0].update_failed.as_ref(), Some(&failed));

    // A v4 probe says no version: the one on record stays. The failure
    // clears when the probe stops reporting it.
    s.record_probe_state("p1", &ReportedState::default())
        .await
        .unwrap();
    let record = s.probe_record("p1").await.unwrap().unwrap();
    assert_eq!(record.probe_version.as_deref(), Some("0.2.0"));
    assert_eq!(record.update_failed, None);
}

async fn one_release_is_approved_or_none(s: &dyn Storage) {
    assert_eq!(
        s.approved_probe_version().await.unwrap(),
        None,
        "none by default"
    );
    let approve = |version: &str| ApprovedVersion {
        version: version.into(),
        approved_by: Some("owner".into()),
        approved_at: now(),
    };
    s.set_approved_probe_version(Some(&approve("0.2.0")))
        .await
        .unwrap();
    s.set_approved_probe_version(Some(&approve("0.3.0")))
        .await
        .unwrap();
    assert_eq!(
        s.approved_probe_version().await.unwrap(),
        Some(approve("0.3.0"))
    );
    s.set_approved_probe_version(None).await.unwrap();
    assert_eq!(s.approved_probe_version().await.unwrap(), None);
}

async fn silences_of_checks_carry_their_company(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &["c1"]).await;
    s.create_silence(SILENCE_CHECK, "c1", "maintenance", None, now())
        .await
        .unwrap();
    s.create_silence(SILENCE_CHECK, "deleted", "old", None, now())
        .await
        .unwrap();

    let mut active = s.active_silences(now()).await.unwrap();
    active.sort_by(|a, b| a.scope_id.cmp(&b.scope_id));
    let companies: Vec<_> = active.iter().map(|x| x.company_id.as_deref()).collect();
    assert_eq!(
        companies,
        [Some("acme"), None],
        "c1, then the deleted check's"
    );
}
