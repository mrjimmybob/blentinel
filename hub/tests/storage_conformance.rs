//! Storage conformance suite (proposal 0011 §7): the contract every storage
//! backend must meet, written once against the `Storage` traits only (no SQL).
//!
//! Each backend gets one `conformance_suite!` line. A backend counts as
//! supported only when the whole suite passes against it.
#![cfg(feature = "ssr")]

use chrono::{DateTime, Duration, Utc};
use common::checks::{CheckId, CheckResult, CheckSpec, ConfigRejection, Health, Metric};
use common::commands::CommandSpec;
use common::models::{PinDigests, ProbeHealth, ProbeProblem, StatusReport, UpdateFailure};
use hub_lib::model::IncidentEventKind;
use hub_lib::model::{
    AcmeAuthority, AcmeChallenge, AcmeSettings, AlertKind, BackupSchedule, ChannelKind,
    CommandState, CompanyScope, Evaluation, MailSecurity, Role, RotationState, RouteRule, Scope,
    Setting, Severity, Status, Thresholds,
};
use hub_lib::model::{CompanyUpdatePolicy, RolloutStage, UpdateMode, UpdateWindow};
use hub_lib::storage::{
    BackupRun, CheckRecord, CommandRecord, NewRotation, NewUser, ProbePins, ReportedState, Rollout,
    RolloutChange, RolloutPaused, SaveOutcome, Storage, StoredAlertSettings, StoredChannel,
    UpgradeCheck, UptimeRange, UserUpdate,
};
use hub_lib::storage::{StoredIncident, StoredIncidentEvent};

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
                a_probes_health_and_silence_are_listed,
                a_rollout_is_started_changed_in_order_and_stopped,
                canaries_and_company_policies_round_trip,
                the_upgrade_check_is_off_by_default_and_keeps_what_it_found,
                silences_of_any_scope_cover_what_is_under_them,
                commands_are_queued_delivered_completed_and_go_with_their_probe,
                only_queued_commands_are_cancelled_and_expired_ones_arent_pending,
                rotations_are_recorded_and_end_once_with_the_pins_probes_hold,
                the_backup_schedule_and_its_last_run_round_trip,
                acme_settings_round_trip_and_attempts_count_failures,
                verdicts_are_stored_counted_and_unknown_once_the_probe_stops,
                thresholds_round_trip_per_level,
                channels_rules_and_alert_settings_round_trip,
                incidents_open_change_close_and_acknowledge_once,
                the_digest_settings_and_its_last_day_round_trip,
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
        command_results: vec![],
        pins: None,
        health: None,
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
        alert_after: Default::default(),
        limits: Default::default(),
        depends_on: None,
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

/// Store a report with the probe's own verdicts (no thresholds).
async fn save(
    s: &dyn Storage,
    report: &StatusReport,
) -> hub_lib::storage::StorageResult<SaveOutcome> {
    let evaluations: Vec<Evaluation> = report
        .results
        .iter()
        .map(|r| Evaluation::of_health(r.status))
        .collect();
    s.save_report(report, &evaluations).await
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
        save(s, &report(probe, company, at, devices)).await.unwrap(),
        SaveOutcome::Stored
    );
    s.record_heartbeat(probe, company, at).await.unwrap();
}

// ---------------------------------------------------------------------------
// Contracts
// ---------------------------------------------------------------------------

async fn reports_are_stored_only_if_newer_than_the_probes_latest(s: &dyn Storage) {
    let r = |at| report("p1", "acme", at, vec![device("a", true)]);
    assert_eq!(save(s, &r(now())).await.unwrap(), SaveOutcome::Stored);

    assert_eq!(
        save(s, &r(now())).await.unwrap(),
        SaveOutcome::NotNewer,
        "same timestamp"
    );
    assert_eq!(
        save(s, &r(now() - Duration::seconds(1))).await.unwrap(),
        SaveOutcome::NotNewer,
        "older"
    );
    assert_eq!(
        save(s, &r(now() + Duration::milliseconds(1)))
            .await
            .unwrap(),
        SaveOutcome::Stored,
        "1 ms newer"
    );
}

async fn replay_ordering_is_per_probe(s: &dyn Storage) {
    assert_eq!(
        save(s, &report("p1", "acme", now(), vec![])).await.unwrap(),
        SaveOutcome::Stored
    );
    assert_eq!(
        save(s, &report("p2", "acme", now(), vec![])).await.unwrap(),
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
    assert_eq!(devices[0].status, Status::Up);
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
    assert_eq!(devices[1].status, Status::Down);
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
        save(s, &report("p1", "acme", t(at), vec![device("a", up)]))
            .await
            .unwrap();
    }
    save(
        s,
        &report(
            "p2",
            "zeta",
            t("2026-03-10T10:05:00Z"),
            vec![device("z", false)],
        ),
    )
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

    s.upsert_alert_state("k", "Down", 0, true, now())
        .await
        .unwrap();
    let state = s.alert_state("k").await.unwrap().unwrap();
    assert_eq!(
        (state.last_status.as_str(), state.last_alert_sent_at),
        ("Down", Some(now()))
    );

    s.upsert_alert_state("k", "Up", 1, false, now() + Duration::minutes(1))
        .await
        .unwrap();
    let state = s.alert_state("k").await.unwrap().unwrap();
    assert_eq!((state.last_status.as_str(), state.streak), ("Up", 1));
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

fn check_scope(id: &str) -> Scope {
    Scope::Check {
        check_id: id.into(),
    }
}

async fn silences_upsert_expire_and_delete(s: &dyn Storage) {
    let first = s
        .create_silence(
            "acme",
            &check_scope("k1"),
            "first",
            Some(now() + Duration::hours(1)),
            now(),
        )
        .await
        .unwrap();
    let again = s
        .create_silence("acme", &check_scope("k1"), "second", None, now())
        .await
        .unwrap();
    assert_eq!(first, again, "same scope updates the existing silence");
    let timed = s
        .create_silence(
            "acme",
            &check_scope("k2"),
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
            .any(|x| x.is_of_check("k1") && x.reason == "second" && x.expires_at.is_none())
    );

    let later = now() + Duration::hours(2);
    let silenced = |check: &'static str| s.is_silenced("acme", "hq", "p1", Some(check), later);
    assert!(!silenced("k2").await.unwrap(), "timed silence expired");
    assert!(silenced("k1").await.unwrap(), "forever silence remains");
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
        timezone: "Europe/Madrid".into(),
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
        canary: false,
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
    named_a.alert_after.fail_after = Some(3);
    s.create_check(&named_a).await.unwrap();

    let checks = s.probe_checks("p1").await.unwrap();
    let names: Vec<_> = checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["a check", "b-id"], "by name");
    assert_eq!(checks[0].company_id, "acme", "from the probe record");
    assert_eq!(checks[0].spec, CheckSpec::Disk { path: None });
    assert_eq!(checks[0].alert_after.fail_after, Some(3));
    assert_eq!(checks[0].alert_after.recover_after, None, "the default");
    assert_eq!(checks[0].updated_by.as_deref(), Some("alice"));
    assert!(s.probe_checks("nobody").await.unwrap().is_empty());

    let mut changed = checks[1].clone();
    changed.name = "Router".into();
    changed.spec = CheckSpec::Tcp {
        host: "10.0.0.1".into(),
        port: 443,
    };
    changed.enabled = false;
    changed.alert_after = hub_lib::model::AlertAfter {
        fail_after: Some(1),
        recover_after: Some(4),
    };
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

async fn a_probes_health_and_silence_are_listed(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &["c1"]).await;
    let listed = &s.probe_records(&CompanyScope::All).await.unwrap()[0];
    assert_eq!(listed.health, None, "none before a v8 report");
    assert!(!listed.silent);

    let health = ProbeHealth {
        started_at: now() - Duration::hours(3),
        queued_reports: 12,
        clock_offset_secs: Some(-75),
        last_error: Some(ProbeProblem {
            at: now() - Duration::minutes(5),
            message: "Can't reach the hub: timed out".into(),
        }),
    };
    let refused = ConfigRejection {
        revision: "r9".into(),
        error: "unknown kind".into(),
    };
    let state = ReportedState {
        config_revision: Some("r1"),
        config_rejected: Some(&refused),
        health: Some(&health),
        ..Default::default()
    };
    s.record_probe_state("p1", &state).await.unwrap();
    // A v7 report says nothing about health: the last one stays.
    let v7 = ReportedState {
        config_revision: Some("r1"),
        config_rejected: Some(&refused),
        ..Default::default()
    };
    s.record_probe_state("p1", &v7).await.unwrap();
    let listed = &s.probe_records(&CompanyScope::All).await.unwrap()[0];
    assert_eq!(listed.health.as_ref(), Some(&health));
    assert_eq!(listed.config_revision.as_deref(), Some("r1"));
    assert_eq!(listed.config_rejected.as_ref(), Some(&refused));
    assert_eq!(listed.config_sync, None, "storage doesn't work it out");

    // Silent once its liveness record expired, reporting again after.
    s.record_heartbeat("p1", "acme", now()).await.unwrap();
    s.expire_silent_probes(300, now() + Duration::seconds(301))
        .await
        .unwrap();
    assert!(s.probe_records(&CompanyScope::All).await.unwrap()[0].silent);
    s.record_heartbeat("p1", "acme", now() + Duration::seconds(400))
        .await
        .unwrap();
    assert!(!s.probe_records(&CompanyScope::All).await.unwrap()[0].silent);
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

async fn rotations_are_recorded_and_end_once_with_the_pins_probes_hold(s: &dyn Storage) {
    let now = t("2026-03-10T12:00:00Z");
    assert!(s.rotations(5).await.unwrap().is_empty());
    let id = s
        .start_rotation(&NewRotation {
            rotates_tls: true,
            rotates_key: false,
            window_days: 14,
            started_by: Some("ann".into()),
            started_at: now,
            deadline_at: now + Duration::days(14),
        })
        .await
        .unwrap();
    let started = &s.rotations(5).await.unwrap()[0];
    assert_eq!((started.id, started.state), (id, RotationState::Announcing));
    assert_eq!((started.rotates_tls, started.rotates_key), (true, false));
    assert!(!started.deadline_warned);
    s.mark_rotation_warned(id).await.unwrap();

    let at = now + Duration::days(2);
    assert!(
        s.end_rotation(id, RotationState::Switched, None, at)
            .await
            .unwrap()
    );
    assert!(
        !s.end_rotation(id, RotationState::Cancelled, Some("bob"), at)
            .await
            .unwrap(),
        "only an announcing rotation ends"
    );
    let ended = &s.rotations(5).await.unwrap()[0];
    assert_eq!(ended.state, RotationState::Switched);
    assert_eq!(
        (ended.ended_by.as_deref(), ended.ended_at),
        (None, Some(at))
    );
    assert!(ended.deadline_warned);

    // Pins: none before a v7 report; a v6 report (no pins) keeps them.
    give_checks(s, "p1", "acme", &["c1"]).await;
    let none = ProbePins {
        probe_id: "p1".into(),
        current: None,
        next: None,
    };
    assert_eq!(s.active_probe_pins().await.unwrap(), vec![none]);
    let pins = PinDigests {
        current: "aa".into(),
        next: Some("bb".into()),
    };
    let state = ReportedState {
        pins: Some(&pins),
        ..Default::default()
    };
    s.record_probe_state("p1", &state).await.unwrap();
    s.record_probe_state("p1", &ReportedState::default())
        .await
        .unwrap();
    let held = ProbePins {
        probe_id: "p1".into(),
        current: Some("aa".into()),
        next: Some("bb".into()),
    };
    assert_eq!(s.active_probe_pins().await.unwrap(), vec![held]);
    s.revoke_probe_record("p1", now).await.unwrap();
    assert!(
        s.active_probe_pins().await.unwrap().is_empty(),
        "active only"
    );
}

async fn the_backup_schedule_and_its_last_run_round_trip(s: &dyn Storage) {
    assert_eq!(s.backup_schedule().await.unwrap(), None, "never set");
    let schedule = BackupSchedule {
        enabled: false,
        hour_utc: 22,
        keep: 30,
    };
    s.set_backup_schedule(&schedule, Some("ann"), t("2026-03-10T12:00:00Z"))
        .await
        .unwrap();
    assert_eq!(s.backup_schedule().await.unwrap(), Some(schedule));

    assert_eq!(s.last_backup_run().await.unwrap(), None);
    for run in [
        BackupRun {
            ran_at: t("2026-03-10T12:00:00Z"),
            error: Some("disk full".into()),
        },
        BackupRun {
            ran_at: t("2026-03-10T12:00:00Z") + Duration::days(1),
            error: None,
        },
    ] {
        s.record_backup_run(&run).await.unwrap();
        assert_eq!(s.last_backup_run().await.unwrap(), Some(run));
    }
}

async fn acme_settings_round_trip_and_attempts_count_failures(s: &dyn Storage) {
    assert_eq!(s.acme_settings().await.unwrap(), None, "never set: off");
    let settings = AcmeSettings {
        enabled: true,
        domains: vec!["hub.example.com".into(), "monitor.example.org".into()],
        email: "ops@example.com".into(),
        authority: AcmeAuthority::Custom,
        directory_url: "https://ca.internal/acme/directory".into(),
        ca_pem: "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----".into(),
        challenge: AcmeChallenge::Http,
        terms_agreed: true,
    };
    s.set_acme_settings(&settings, Some("ann"), now())
        .await
        .unwrap();
    assert_eq!(s.acme_settings().await.unwrap(), Some(settings.clone()));
    let off = AcmeSettings {
        enabled: false,
        ..settings
    };
    s.set_acme_settings(&off, None, now()).await.unwrap();
    assert_eq!(s.acme_settings().await.unwrap(), Some(off));

    assert_eq!(s.acme_status().await.unwrap(), None, "never asked");
    let first = now();
    s.record_acme_attempt(first, Some("connection refused"))
        .await
        .unwrap();
    let second = first + Duration::hours(1);
    s.record_acme_attempt(second, Some("timed out"))
        .await
        .unwrap();
    let status = s.acme_status().await.unwrap().unwrap();
    assert_eq!(
        (status.last_attempt_at, status.last_error.as_deref()),
        (second, Some("timed out"))
    );
    assert_eq!((status.failures, status.last_issued_at), (2, None));

    let issued = second + Duration::hours(2);
    s.record_acme_attempt(issued, None).await.unwrap();
    let status = s.acme_status().await.unwrap().unwrap();
    assert_eq!(
        (status.failures, status.last_error, status.last_issued_at),
        (0, None, Some(issued)),
        "a certificate resets the failures"
    );
    let later = issued + Duration::days(60);
    s.record_acme_attempt(later, Some("rate limited"))
        .await
        .unwrap();
    let status = s.acme_status().await.unwrap().unwrap();
    assert_eq!(
        (status.failures, status.last_issued_at),
        (1, Some(issued)),
        "a failure keeps when the last one was issued"
    );
}

async fn verdicts_are_stored_counted_and_unknown_once_the_probe_stops(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &["a", "b", "c"]).await;
    let r = report(
        "p1",
        "acme",
        now(),
        vec![device("a", true), device("b", true), device("c", false)],
    );
    let evaluations = [
        Evaluation::of_health(common::checks::Health::Up),
        Evaluation {
            status: Status::Warning,
            reason: Some("disk usage 92% is over the warning limit".into()),
            in_maintenance: false,
            behind: None,
        },
        Evaluation::of_health(common::checks::Health::Down),
    ];
    assert_eq!(
        s.save_report(&r, &evaluations).await.unwrap(),
        SaveOutcome::Stored
    );
    s.record_heartbeat("p1", "acme", now()).await.unwrap();

    let devices = s.probe_devices("p1").await.unwrap();
    let statuses: Vec<_> = devices
        .iter()
        .map(|d| (d.status, d.reason.as_deref()))
        .collect();
    assert_eq!(
        statuses,
        [
            (Status::Up, None),
            (
                Status::Warning,
                Some("disk usage 92% is over the warning limit")
            ),
            (Status::Down, None)
        ]
    );
    let company = &s.dashboard_companies(&CompanyScope::All).await.unwrap()[0];
    assert_eq!(
        (
            company.devices_up,
            company.devices_warning,
            company.devices_down
        ),
        (1, 1, 1)
    );
    assert_eq!(company.severity, Severity::Critical);
    let probe = &s.company_probes("acme").await.unwrap()[0];
    assert_eq!(probe.devices_warning, 1);
    let uptime = s
        .company_uptime("acme", UptimeRange::Hours24, now())
        .await
        .unwrap();
    assert_eq!(
        (uptime[0].up_count, uptime[0].down_count),
        (2, 1),
        "a warning works: it counts as up"
    );

    // The probe stops: its last results are history, nobody knows now.
    s.expire_silent_probes(60, now() + Duration::minutes(10))
        .await
        .unwrap();
    let devices = s.probe_devices("p1").await.unwrap();
    assert!(devices.iter().all(|d| d.status == Status::Unknown));
}

async fn thresholds_round_trip_per_level(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &[]).await;
    assert_eq!(s.threshold_defaults().await.unwrap(), None, "never set");
    assert_eq!(
        s.company_thresholds("acme").await.unwrap(),
        Thresholds::default()
    );
    assert_eq!(
        s.site_thresholds("acme", "main").await.unwrap(),
        Thresholds::default()
    );

    let mut hub = Thresholds::default();
    hub.cpu.warning = Some(Setting::At(80.0));
    hub.latency.down = Some(Setting::Off);
    s.set_threshold_defaults(&hub, Some("ann"), now())
        .await
        .unwrap();
    assert_eq!(s.threshold_defaults().await.unwrap(), Some(hub));

    let mut company = Thresholds::default();
    company.disk.down = Some(Setting::At(99.5));
    assert!(s.set_company_thresholds("acme", &company).await.unwrap());
    assert_eq!(s.company_thresholds("acme").await.unwrap(), company);
    assert!(!s.set_company_thresholds("nobody", &company).await.unwrap());

    let mut site = Thresholds::default();
    site.memory.warning = Some(Setting::At(70.0));
    assert!(s.set_site_thresholds("acme", "main", &site).await.unwrap());
    assert_eq!(s.site_thresholds("acme", "main").await.unwrap(), site);
    assert!(
        !s.set_site_thresholds("acme", "nowhere", &site)
            .await
            .unwrap()
    );

    // Cleared: back to nothing set.
    assert!(
        s.set_company_thresholds("acme", &Thresholds::default())
            .await
            .unwrap()
    );
    assert_eq!(
        s.company_thresholds("acme").await.unwrap(),
        Thresholds::default()
    );
}

fn rollout(version: &str) -> Rollout {
    Rollout {
        version: version.into(),
        approved_by: Some("owner".into()),
        approved_at: now(),
        stage: RolloutStage::Canary,
        soak_minutes: 60,
        canaries_ready_at: None,
        promoted_at: None,
        promoted_by: None,
        paused: None,
    }
}

async fn a_rollout_is_started_changed_in_order_and_stopped(s: &dyn Storage) {
    assert_eq!(s.rollout().await.unwrap(), None, "none by default");
    s.set_rollout(Some(&rollout("0.2.0"))).await.unwrap();
    s.set_rollout(Some(&rollout("0.3.0"))).await.unwrap();
    assert_eq!(s.rollout().await.unwrap(), Some(rollout("0.3.0")));

    let later = now() + Duration::minutes(5);
    // A change for another version does nothing.
    assert!(
        !s.change_rollout("0.2.0", &RolloutChange::Resume, later)
            .await
            .unwrap()
    );
    let ready = RolloutChange::CanariesReady(Some(later));
    assert!(s.change_rollout("0.3.0", &ready, later).await.unwrap());
    assert_eq!(
        s.rollout().await.unwrap().unwrap().canaries_ready_at,
        Some(later)
    );

    // Paused once; resumed once.
    let pause = RolloutChange::Pause {
        by: None,
        reason: "canary failed",
    };
    assert!(s.change_rollout("0.3.0", &pause, later).await.unwrap());
    assert!(!s.change_rollout("0.3.0", &pause, later).await.unwrap());
    assert_eq!(
        s.rollout().await.unwrap().unwrap().paused,
        Some(RolloutPaused {
            at: later,
            by: None,
            reason: "canary failed".into()
        })
    );
    // The hub doesn't promote a paused rollout; an admin may (and resumes it).
    let by_hub = RolloutChange::Promote { by: None };
    assert!(!s.change_rollout("0.3.0", &by_hub, later).await.unwrap());
    let by_admin = RolloutChange::Promote { by: Some("owner") };
    assert!(s.change_rollout("0.3.0", &by_admin, later).await.unwrap());
    let promoted = s.rollout().await.unwrap().unwrap();
    assert_eq!(promoted.stage, RolloutStage::All);
    assert_eq!(promoted.promoted_at, Some(later));
    assert_eq!(promoted.promoted_by.as_deref(), Some("owner"));
    assert_eq!(promoted.paused, None);
    // Only once, and canary readiness no longer applies.
    assert!(!s.change_rollout("0.3.0", &by_admin, later).await.unwrap());
    assert!(!s.change_rollout("0.3.0", &ready, later).await.unwrap());

    assert!(s.change_rollout("0.3.0", &pause, later).await.unwrap());
    assert!(
        s.change_rollout("0.3.0", &RolloutChange::Resume, later)
            .await
            .unwrap()
    );
    assert_eq!(s.rollout().await.unwrap().unwrap().paused, None);

    s.set_rollout(None).await.unwrap();
    assert_eq!(s.rollout().await.unwrap(), None);
}

async fn canaries_and_company_policies_round_trip(s: &dyn Storage) {
    place(s, "acme", "hq").await;
    place(s, "globex", "hq").await;
    s.save_probe_record(&record("p1", "acme", "hq"))
        .await
        .unwrap();
    assert!(!s.probe_record("p1").await.unwrap().unwrap().canary);
    assert!(s.set_probe_canary("p1", true).await.unwrap());
    assert!(!s.set_probe_canary("nobody", true).await.unwrap());
    assert!(s.probe_record("p1").await.unwrap().unwrap().canary);
    let listed = s.probe_records(&CompanyScope::All).await.unwrap();
    assert!(listed[0].canary);
    // Enrolling again keeps it.
    s.save_probe_record(&record("p1", "acme", "hq"))
        .await
        .unwrap();
    assert!(s.probe_record("p1").await.unwrap().unwrap().canary);

    assert_eq!(
        s.company_update_policy("acme").await.unwrap(),
        CompanyUpdatePolicy::default()
    );
    let pinned = CompanyUpdatePolicy {
        mode: UpdateMode::Pin,
        version: Some("0.2.0".into()),
        window: Some(UpdateWindow {
            start: "22:00".into(),
            end: "04:00".into(),
        }),
    };
    s.set_company_update_policy("acme", &pinned, Some("owner"), now())
        .await
        .unwrap();
    let off = CompanyUpdatePolicy {
        mode: UpdateMode::Off,
        ..Default::default()
    };
    s.set_company_update_policy("globex", &off, None, now())
        .await
        .unwrap();
    assert_eq!(s.company_update_policy("acme").await.unwrap(), pinned);
    assert_eq!(
        s.company_update_policies().await.unwrap(),
        vec![("acme".to_string(), pinned), ("globex".to_string(), off)]
    );
    // The default forgets it.
    s.set_company_update_policy("globex", &CompanyUpdatePolicy::default(), None, now())
        .await
        .unwrap();
    assert_eq!(s.company_update_policies().await.unwrap().len(), 1);

    assert_eq!(
        s.site_timezone("acme", "hq").await.unwrap().as_deref(),
        Some("UTC")
    );
    assert_eq!(s.site_timezone("acme", "nowhere").await.unwrap(), None);
    let mut zones = s.site_timezones().await.unwrap();
    zones.sort();
    assert_eq!(
        zones,
        [
            ("acme".to_string(), "hq".to_string(), "UTC".to_string()),
            ("globex".to_string(), "hq".to_string(), "UTC".to_string()),
        ]
    );
}

async fn the_upgrade_check_is_off_by_default_and_keeps_what_it_found(s: &dyn Storage) {
    assert_eq!(s.upgrade_check().await.unwrap(), UpgradeCheck::default());
    assert!(!UpgradeCheck::default().enabled);

    // "Check now" works while it is off, and leaves it off.
    let first = now();
    s.record_upgrade_check(first, Some("0.5.0"), None)
        .await
        .unwrap();
    let found = s.upgrade_check().await.unwrap();
    assert!(!found.enabled);
    assert_eq!(found.latest_version.as_deref(), Some("0.5.0"));
    assert_eq!(found.checked_at, Some(first));

    s.set_upgrade_check_enabled(true, Some("owner"), now())
        .await
        .unwrap();
    // A failed check keeps the release found before.
    let later = now() + Duration::days(1);
    s.record_upgrade_check(later, None, Some("can't download"))
        .await
        .unwrap();
    assert_eq!(
        s.upgrade_check().await.unwrap(),
        UpgradeCheck {
            enabled: true,
            checked_at: Some(later),
            latest_version: Some("0.5.0".into()),
            error: Some("can't download".into()),
        }
    );
    s.set_upgrade_check_enabled(false, None, now())
        .await
        .unwrap();
    assert!(!s.upgrade_check().await.unwrap().enabled);
}

async fn silences_of_any_scope_cover_what_is_under_them(s: &dyn Storage) {
    let at = |scope: Scope| async move {
        s.create_silence("acme", &scope, "r", None, now())
            .await
            .unwrap()
    };
    let covers = |site: &'static str, probe: &'static str, check: Option<&'static str>| {
        s.is_silenced("acme", site, probe, check, now())
    };
    assert!(
        !covers("hq", "p1", Some("c1")).await.unwrap(),
        "nothing yet"
    );

    let site = at(Scope::Site {
        site_id: "hq".into(),
    })
    .await;
    assert!(covers("hq", "p1", Some("c1")).await.unwrap());
    assert!(
        covers("hq", "p1", None).await.unwrap(),
        "the probe itself too"
    );
    assert!(
        !covers("branch", "p2", Some("c2")).await.unwrap(),
        "another site"
    );
    assert!(
        !s.is_silenced("zeta", "hq", "p1", Some("c1"), now())
            .await
            .unwrap(),
        "another company's site called hq"
    );
    s.delete_silence(site).await.unwrap();

    let probe = at(Scope::Probe {
        probe_id: "p1".into(),
    })
    .await;
    assert!(covers("hq", "p1", Some("c1")).await.unwrap());
    assert!(!covers("hq", "p2", Some("c2")).await.unwrap());
    s.delete_silence(probe).await.unwrap();

    at(Scope::Company).await;
    assert!(covers("branch", "p2", None).await.unwrap());
    let listed = s.active_silences(now()).await.unwrap();
    assert_eq!(
        (listed[0].company_id.as_str(), &listed[0].scope),
        ("acme", &Scope::Company)
    );
}

fn command(id: &str, probe: &str, created: DateTime<Utc>) -> CommandRecord {
    CommandRecord {
        id: id.into(),
        probe_id: probe.into(),
        company_id: String::new(), // read from the probe record
        spec: CommandSpec::RunChecks,
        created_by: Some("alice".into()),
        created_at: created,
        expires_at: created + Duration::minutes(15),
        state: CommandState::Queued,
        delivered_at: None,
        completed_at: None,
        result: None,
    }
}

async fn commands_are_queued_delivered_completed_and_go_with_their_probe(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &[]).await;
    give_checks(s, "p2", "acme", &[]).await;
    s.create_command(&command("old", "p1", now()))
        .await
        .unwrap();
    s.create_command(&command("new", "p1", now() + Duration::minutes(1)))
        .await
        .unwrap();
    s.create_command(&command("theirs", "p2", now()))
        .await
        .unwrap();

    let listed = s.probe_commands("p1", 10).await.unwrap();
    let ids: Vec<_> = listed.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["new", "old"], "newest first");
    assert_eq!(s.probe_commands("p1", 1).await.unwrap().len(), 1, "limit");
    let stored = s.command("old").await.unwrap().expect("stored");
    assert_eq!(stored.company_id, "acme", "from the probe record");
    assert_eq!(stored.spec, CommandSpec::RunChecks);
    assert_eq!(stored.created_by.as_deref(), Some("alice"));

    let pending = s.pending_commands("p1", now(), 10).await.unwrap();
    let ids: Vec<_> = pending.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["old", "new"], "oldest first");
    assert_eq!(s.pending_commands("p1", now(), 1).await.unwrap().len(), 1);

    let at = now() + Duration::minutes(2);
    s.mark_commands_delivered(&["old".into()], at)
        .await
        .unwrap();
    let delivered = s.command("old").await.unwrap().unwrap();
    assert_eq!(delivered.state, CommandState::Delivered);
    assert_eq!(delivered.delivered_at, Some(at));
    // Delivering again keeps the first time.
    s.mark_commands_delivered(&["old".into()], at + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(
        s.command("old").await.unwrap().unwrap().delivered_at,
        Some(at)
    );
    assert_eq!(s.pending_commands("p1", at, 10).await.unwrap().len(), 2);

    // Only the probe it's for can complete it, once.
    let done = at + Duration::minutes(1);
    assert!(
        !s.complete_command("p2", "old", CommandState::Succeeded, "x", done)
            .await
            .unwrap()
    );
    assert!(
        s.complete_command("p1", "old", CommandState::Succeeded, "ran 1 check", done)
            .await
            .unwrap()
    );
    assert!(
        !s.complete_command("p1", "old", CommandState::Failed, "late", done)
            .await
            .unwrap()
    );
    let completed = s.command("old").await.unwrap().unwrap();
    assert_eq!(completed.state, CommandState::Succeeded);
    assert_eq!(completed.result.as_deref(), Some("ran 1 check"));
    assert_eq!(completed.completed_at, Some(done));
    // A queued one can complete too (its answer may have been lost).
    assert!(
        s.complete_command("p1", "new", CommandState::Failed, "no", done)
            .await
            .unwrap()
    );
    assert!(s.pending_commands("p1", done, 10).await.unwrap().is_empty());

    // A probe's commands go with it.
    s.delete_probe_record("p1").await.unwrap();
    assert!(s.command("old").await.unwrap().is_none());
    assert!(s.command("theirs").await.unwrap().is_some());
}

async fn only_queued_commands_are_cancelled_and_expired_ones_arent_pending(s: &dyn Storage) {
    give_checks(s, "p1", "acme", &[]).await;
    s.create_command(&command("a", "p1", now())).await.unwrap();
    s.create_command(&command("b", "p1", now())).await.unwrap();
    s.mark_commands_delivered(&["b".into()], now())
        .await
        .unwrap();

    let at = now() + Duration::minutes(1);
    assert!(s.cancel_command("a", at).await.unwrap());
    assert!(
        !s.cancel_command("a", at).await.unwrap(),
        "already cancelled"
    );
    assert!(!s.cancel_command("b", at).await.unwrap(), "already sent");
    assert!(!s.cancel_command("nope", at).await.unwrap());
    let cancelled = s.command("a").await.unwrap().unwrap();
    assert_eq!(cancelled.state, CommandState::Cancelled);
    assert_eq!(cancelled.completed_at, Some(at));

    let pending = s.pending_commands("p1", at, 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    // At its expiry it is no longer pending (but stays listed).
    let expiry = now() + Duration::minutes(15);
    assert!(
        s.pending_commands("p1", expiry, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(s.probe_commands("p1", 10).await.unwrap().len(), 2);
}

async fn channels_rules_and_alert_settings_round_trip(s: &dyn Storage) {
    assert_eq!(s.alert_settings().await.unwrap(), None, "never set");
    let settings = StoredAlertSettings {
        hub_url: "https://hub.example.com".into(),
        smtp_server: "smtp.example.com".into(),
        smtp_port: 465,
        smtp_security: MailSecurity::Tls,
        smtp_username: "hub".into(),
        smtp_from: "hub@example.com".into(),
        smtp_password_sealed: Some("sealed".into()),
    };
    s.set_alert_settings(&settings, Some("owner"), now())
        .await
        .unwrap();
    s.set_alert_settings(&settings, None, now()).await.unwrap();
    assert_eq!(s.alert_settings().await.unwrap(), Some(settings));

    let channel = |name: &str| StoredChannel {
        id: 0,
        name: name.into(),
        kind: ChannelKind::Webhook,
        enabled: true,
        target: "https://example.com/hook".into(),
        secret_sealed: Some("sealed".into()),
        last_delivery_at: None,
        last_error: None,
    };
    let b = s.create_channel(&channel("b"), now()).await.unwrap();
    let a = s.create_channel(&channel("a"), now()).await.unwrap();
    assert_ne!(a, b);
    let names: Vec<String> = s
        .channels()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["a", "b"], "by name");

    let mut changed = s.channel(a).await.unwrap().unwrap();
    changed.kind = ChannelKind::Slack;
    changed.enabled = false;
    changed.secret_sealed = None;
    assert!(s.update_channel(&changed).await.unwrap());
    assert_eq!(s.channel(a).await.unwrap(), Some(changed.clone()));
    assert!(
        !s.update_channel(&StoredChannel { id: 999, ..changed })
            .await
            .unwrap()
    );

    let at = now() + Duration::minutes(1);
    s.record_delivery(a, at, Some("refused")).await.unwrap();
    let failed = s.channel(a).await.unwrap().unwrap();
    assert_eq!(
        (failed.last_delivery_at, failed.last_error.as_deref()),
        (Some(at), Some("refused"))
    );
    s.record_delivery(a, at, None).await.unwrap();
    assert_eq!(s.channel(a).await.unwrap().unwrap().last_error, None);

    let rule = |name: &str, channels: Vec<i64>| RouteRule {
        name: name.into(),
        companies: vec!["acme".into()],
        sites: vec!["acme/hq".into()],
        kinds: vec![AlertKind::Down],
        check_kinds: vec!["ping".into()],
        channels,
        stop: true,
        ..RouteRule::default()
    };
    assert!(s.route_rules().await.unwrap().is_empty());
    let rules = vec![rule("second", vec![b]), rule("first", vec![a, b])];
    s.set_route_rules(&rules).await.unwrap();
    assert_eq!(s.route_rules().await.unwrap(), rules, "in the order given");
    s.set_route_rules(&rules[..1]).await.unwrap();
    assert_eq!(s.route_rules().await.unwrap(), rules[..1], "replaced");

    assert!(s.delete_channel(a).await.unwrap());
    assert!(!s.delete_channel(a).await.unwrap());
    assert_eq!(s.channel(a).await.unwrap(), None);
}

async fn incidents_open_change_close_and_acknowledge_once(s: &dyn Storage) {
    let incident = |company: &str, check: Option<&str>| StoredIncident {
        id: 0,
        company_id: company.into(),
        company_name: format!("{company} Ltd"),
        site_id: "hq".into(),
        site_name: "Head office".into(),
        probe_id: format!("{company}-probe"),
        probe_name: "probe".into(),
        check_id: check.map(str::to_string),
        check_name: check.map(|c| format!("{c} name")),
        severity: Status::Warning,
        status: Status::Warning,
        opened_at: now(),
        closed_at: None,
        acknowledged_by: None,
        acknowledged_at: None,
        announced: true,
        reminders: 0,
        last_reminded_at: None,
        escalated_at: None,
    };
    let event = |kind, minutes: i64, by: Option<&str>, text: &str| StoredIncidentEvent {
        at: now() + Duration::minutes(minutes),
        kind,
        username: by.map(str::to_string),
        text: text.into(),
    };
    let opened = event(IncidentEventKind::Opened, 0, None, "Warning: disk 95%");
    let a = s
        .open_incident(&incident("acme", Some("c1")), &opened)
        .await
        .unwrap();
    let probe = s
        .open_incident(&incident("acme", None), &opened)
        .await
        .unwrap();
    let z = s
        .open_incident(&incident("zeta", Some("c9")), &opened)
        .await
        .unwrap();
    assert!(
        s.open_incident(&incident("acme", Some("c1")), &opened)
            .await
            .is_err(),
        "one open incident per check"
    );
    assert_eq!(
        s.open_incident_of("acme-probe", Some("c1"))
            .await
            .unwrap()
            .unwrap()
            .id,
        a
    );
    assert_eq!(
        s.open_incident_of("acme-probe", None)
            .await
            .unwrap()
            .unwrap()
            .id,
        probe
    );
    assert_eq!(
        s.open_incident_of("acme-probe", Some("c2")).await.unwrap(),
        None
    );

    // Down, then Warning again: the severity stays Down.
    s.change_incident(
        a,
        Status::Down,
        &event(IncidentEventKind::Changed, 1, None, "Down"),
    )
    .await
    .unwrap();
    s.change_incident(
        a,
        Status::Warning,
        &event(IncidentEventKind::Changed, 2, None, "Warning"),
    )
    .await
    .unwrap();
    let changed = s.incident(a).await.unwrap().unwrap();
    assert_eq!(
        (changed.status, changed.severity),
        (Status::Warning, Status::Down)
    );

    let ack = event(IncidentEventKind::Acknowledged, 3, Some("alice"), "on it");
    assert!(s.acknowledge_incident(a, &ack).await.unwrap());
    assert!(!s.acknowledge_incident(a, &ack).await.unwrap(), "once");
    s.add_incident_event(a, &event(IncidentEventKind::Note, 4, Some("bob"), "PSU"))
        .await
        .unwrap();
    let closed = event(IncidentEventKind::Closed, 5, None, "Recovered");
    assert!(s.close_incident(a, &closed).await.unwrap());
    assert!(
        !s.close_incident(a, &closed).await.unwrap(),
        "closed already"
    );

    let a_now = s.incident(a).await.unwrap().unwrap();
    assert_eq!(a_now.closed_at, Some(now() + Duration::minutes(5)));
    assert_eq!(
        (a_now.acknowledged_by.as_deref(), a_now.acknowledged_at),
        (Some("alice"), Some(now() + Duration::minutes(3)))
    );
    let kinds: Vec<IncidentEventKind> = s
        .incident_events(a)
        .await
        .unwrap()
        .iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            IncidentEventKind::Opened,
            IncidentEventKind::Changed,
            IncidentEventKind::Changed,
            IncidentEventKind::Acknowledged,
            IncidentEventKind::Note,
            IncidentEventKind::Closed,
        ]
    );
    // A new outage of the same check opens a new one.
    let again = s
        .open_incident(&incident("acme", Some("c1")), &opened)
        .await
        .unwrap();
    assert_ne!(again, a);

    let ids = |list: Vec<StoredIncident>| list.into_iter().map(|i| i.id).collect::<Vec<_>>();
    let all = CompanyScope::All;
    assert_eq!(ids(s.incidents(&all, Some(false), 10).await.unwrap()), [a]);
    assert_eq!(
        ids(s.incidents(&all, Some(true), 10).await.unwrap()),
        [again, z, probe]
    );
    assert_eq!(s.incidents(&all, None, 10).await.unwrap().len(), 4);
    assert_eq!(s.incidents(&all, None, 2).await.unwrap().len(), 2);
    let only_zeta = CompanyScope::Only(["zeta".to_string()].into());
    assert_eq!(ids(s.incidents(&only_zeta, None, 10).await.unwrap()), [z]);

    // Open, unacknowledged and announced: what reminders look at (D-69).
    assert_eq!(
        ids(s.unacknowledged_incidents().await.unwrap()),
        [probe, z, again]
    );
    let reminder = event(
        IncidentEventKind::Reminded,
        30,
        None,
        "Reminder 1 to on-call",
    );
    s.record_reminder(z, &reminder).await.unwrap();
    s.record_reminder(z, &reminder).await.unwrap();
    let escalation = event(IncidentEventKind::Escalated, 60, None, "escalated");
    assert!(s.record_escalation(z, &escalation).await.unwrap());
    assert!(!s.record_escalation(z, &escalation).await.unwrap(), "once");
    let z_now = s.incident(z).await.unwrap().unwrap();
    assert_eq!(
        (z_now.reminders, z_now.last_reminded_at, z_now.escalated_at),
        (
            2,
            Some(now() + Duration::minutes(30)),
            Some(now() + Duration::minutes(60))
        )
    );
    let silenced = StoredIncident {
        announced: false,
        ..incident("zeta", Some("c8"))
    };
    s.open_incident(&silenced, &opened).await.unwrap();
    assert_eq!(
        ids(s.unacknowledged_incidents().await.unwrap()),
        [probe, z, again],
        "not one nobody was told about"
    );
}

async fn the_digest_settings_and_its_last_day_round_trip(s: &dyn Storage) {
    assert_eq!(s.digest_settings().await.unwrap(), None);
    assert!(
        !s.record_digest_sent("2026-10-08").await.unwrap(),
        "nothing to send"
    );
    let settings = hub_lib::model::DigestSettings {
        hour: 7,
        timezone: "Europe/Madrid".into(),
        channels: vec![3, 1],
    };
    s.set_digest_settings(&settings, Some("owner"), now())
        .await
        .unwrap();
    assert_eq!(
        s.digest_settings().await.unwrap(),
        Some((settings.clone(), None))
    );

    assert!(s.record_digest_sent("2026-10-08").await.unwrap());
    assert!(
        !s.record_digest_sent("2026-10-08").await.unwrap(),
        "once a day"
    );
    // Saving again keeps the day it went.
    s.set_digest_settings(&settings, None, now()).await.unwrap();
    assert_eq!(
        s.digest_settings().await.unwrap().unwrap().1.as_deref(),
        Some("2026-10-08")
    );
    assert!(s.record_digest_sent("2026-10-09").await.unwrap());
}
