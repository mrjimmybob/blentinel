//! Backups the hub makes by itself (M2-12, D-60): the daily schedule, "Back
//! up now" and the admin API, and the backup before a schema upgrade, put
//! back when the upgrade fails.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use hub_lib::config;
use hub_lib::model::{BackupKind, BackupOverview, BackupSchedule, Role};
use hub_lib::ops::{backup, upgrade};
use hub_lib::service::backups;
use hub_lib::storage;
use serde_json::json;
use support::*;

// ---------------------------------------------------------------------------
// The schedule
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_daily_backup_runs_once_a_day_and_keeps_the_newest() {
    let hub = TestHub::new(&[]).await;
    let config_path = hub.config_path();
    let config = config::load_from(&config_path).unwrap();
    let store = hub.store();
    let schedule = BackupSchedule {
        keep: 2,
        ..BackupSchedule::default()
    };
    backups::set_schedule(store, &schedule, None, hub.now())
        .await
        .unwrap();

    let mut now = hub.now(); // 12:00, after today's 03:00: due (never ran)
    let mut made = Vec::new();
    for _ in 0..3 {
        let file = backups::run_if_due(store, &config, &config_path, now)
            .await
            .unwrap()
            .expect("due")
            .expect("written");
        assert_eq!(file.kind, BackupKind::Scheduled);
        made.push(file.name);
        let again = backups::run_if_due(store, &config, &config_path, now + Duration::hours(1));
        assert!(again.await.unwrap().is_none(), "once a day");
        now += Duration::days(1);
    }

    let kept: Vec<String> = backups::list(&config.backups_dir())
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(
        kept,
        vec![made[2].clone(), made[1].clone()],
        "newest 2, newest first"
    );
    assert!(
        backup::manifest_of(&config.backups_dir().join(&made[2])).is_ok(),
        "a backup `hub restore` reads"
    );
}

#[tokio::test]
async fn a_switched_off_schedule_makes_no_backup() {
    let hub = TestHub::new(&[]).await;
    let config_path = hub.config_path();
    let config = config::load_from(&config_path).unwrap();
    let off = BackupSchedule {
        enabled: false,
        ..BackupSchedule::default()
    };
    backups::set_schedule(hub.store(), &off, None, hub.now())
        .await
        .unwrap();
    let ran = backups::run_if_due(hub.store(), &config, &config_path, hub.now());
    assert!(ran.await.unwrap().is_none());
    assert!(backups::list(&config.backups_dir()).unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_backup_is_recorded_and_waits_for_the_next_day() {
    let hub = TestHub::new(&[]).await;
    let config_path = hub.config_path();
    let mut config = config::load_from(&config_path).unwrap();
    // The backups folder can't be created: a file is in the way.
    let blocked = hub.dir().join("blocked");
    std::fs::write(&blocked, b"a file").unwrap();
    config.backups.path = blocked.join("backups").display().to_string();

    let ran = backups::run_if_due(hub.store(), &config, &config_path, hub.now())
        .await
        .unwrap()
        .expect("due");
    assert!(ran.is_err());
    let overview = backups::overview(hub.store(), &config, hub.now())
        .await
        .unwrap();
    assert!(overview.last_error.is_some(), "the page shows why");
    let retried = backups::run_if_due(hub.store(), &config, &config_path, hub.now());
    assert!(retried.await.unwrap().is_none(), "no retry every minute");
}

// ---------------------------------------------------------------------------
// The admin API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn back_up_now_lists_and_deletes_backups_through_the_api() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let overview: BackupOverview = hub.get_json("/api/admin/backups").await;
    assert_eq!(overview.schedule, BackupSchedule::default());
    assert!(overview.backups.is_empty());
    assert!(overview.next_at.is_some(), "on by default");
    assert!(overview.folder.ends_with("backups"));

    let made = hub
        .post_json("/api/admin/backups", Some(&owner), json!({}))
        .await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.text());
    let name = made.json::<serde_json::Value>()["name"]
        .as_str()
        .unwrap()
        .to_string();
    let overview: BackupOverview = hub.get_json("/api/admin/backups").await;
    assert_eq!(overview.backups.len(), 1);
    assert_eq!(overview.backups[0].name, name);
    assert_eq!(overview.backups[0].kind, BackupKind::Manual);
    assert!(overview.backups[0].size_bytes > 0);

    let twice = hub
        .post_json("/api/admin/backups", Some(&owner), json!({}))
        .await;
    assert_eq!(twice.status, StatusCode::CONFLICT, "the same second");

    for bad in ["..%2Fblentinel.db", "blentinel-hub-20200101-000000.tar.gz"] {
        let refused = hub
            .post_json(
                &format!("/api/admin/backups/{bad}/delete"),
                Some(&owner),
                json!({}),
            )
            .await;
        assert_eq!(refused.status, StatusCode::NOT_FOUND, "{bad}");
    }
    assert!(hub.dir().join("blentinel.db").exists());

    let deleted = hub
        .post_json(
            &format!("/api/admin/backups/{name}/delete"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    let overview: BackupOverview = hub.get_json("/api/admin/backups").await;
    assert!(overview.backups.is_empty());

    let actions: Vec<String> = hub
        .get_json::<Vec<serde_json::Value>>("/api/audit?action=backup")
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        actions.contains(&"backup.create".to_string()),
        "{actions:?}"
    );
    assert!(actions.contains(&"backup.delete".to_string()));
}

#[tokio::test]
async fn the_schedule_is_set_through_the_api_and_validated() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let path = "/api/admin/backups/schedule";

    let set = hub
        .post_json(
            path,
            Some(&owner),
            json!({ "enabled": true, "hour_utc": 22, "keep": 14 }),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text());
    let overview: BackupOverview = hub.get_json("/api/admin/backups").await;
    assert_eq!(
        overview.schedule,
        BackupSchedule {
            enabled: true,
            hour_utc: 22,
            keep: 14,
        }
    );
    // Never ran, and 22:00 yesterday has passed: due now (12:00).
    assert_eq!(overview.next_at.as_deref(), Some("2026-03-10 12:00 UTC"));

    for bad in [
        json!({ "enabled": true, "hour_utc": 24, "keep": 7 }),
        json!({ "enabled": true, "hour_utc": 3, "keep": 0 }),
    ] {
        let refused = hub.post_json(path, Some(&owner), bad).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.error_code(), "invalid_schedule");
    }

    let viewer = hub.login_as(Role::Viewer).await;
    let forbidden = hub
        .post_json("/api/admin/backups", Some(&viewer), json!({}))
        .await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
}

/// Make a backup through the API and return its name.
async fn back_up_now(hub: &TestHub, cookie: &str) -> String {
    let made = hub
        .post_json("/api/admin/backups", Some(cookie), json!({}))
        .await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.text());
    made.json::<serde_json::Value>()["name"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn ask_download(hub: &TestHub, cookie: &str, name: &str, password: &str) -> TestResponse {
    hub.post_json(
        &format!("/api/admin/backups/{name}/download"),
        Some(cookie),
        json!({ "password": password }),
    )
    .await
}

#[tokio::test]
async fn a_backup_downloads_once_after_the_password_is_given_again() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let name = back_up_now(&hub, &owner).await;

    let wrong = ask_download(&hub, &owner, &name, "not-my-password").await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert_eq!(wrong.error_code(), "wrong_password");

    let link = ask_download(&hub, &owner, &name, PASSWORD).await;
    assert_eq!(link.status, StatusCode::OK, "{}", link.text());
    let url = link.json::<serde_json::Value>()["url"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(url.starts_with("/api/admin/backups/download/"), "{url}");

    let file = hub.get(&url, Some(&owner)).await;
    assert_eq!(file.status, StatusCode::OK);
    assert!(
        file.headers[axum::http::header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains(&name)
    );
    let on_disk = std::fs::read(hub.dir().join("backups").join(&name)).unwrap();
    assert_eq!(file.body.as_ref(), on_disk.as_slice(), "the backup itself");
    let again = hub.get(&url, Some(&owner)).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "a link works once");

    let missing = ask_download(
        &hub,
        &owner,
        "blentinel-hub-20200101-000000.tar.gz",
        PASSWORD,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let actions: Vec<String> = hub
        .get_json::<Vec<serde_json::Value>>("/api/audit?action=backup")
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        actions.contains(&"backup.download".to_string()),
        "{actions:?}"
    );
    assert!(actions.contains(&"backup.download_failed".to_string()));
}

#[tokio::test]
async fn a_download_link_is_only_for_the_user_who_gave_the_password() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let admin = hub.login_as(Role::Admin).await;
    let name = back_up_now(&hub, &owner).await;
    let link = ask_download(&hub, &owner, &name, PASSWORD).await;
    let url = link.json::<serde_json::Value>()["url"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        hub.get(&url, Some(&admin)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        hub.get(&url, None).await.status,
        StatusCode::UNAUTHORIZED,
        "a session is needed too"
    );
}

#[tokio::test]
async fn guessing_the_password_for_a_download_is_slowed_down_like_a_login() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let name = back_up_now(&hub, &owner).await;
    for _ in 0..=hub_lib::service::login_limit::FREE_FAILURES {
        let wrong = ask_download(&hub, &owner, &name, "guess").await;
        assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    }
    let locked = ask_download(&hub, &owner, &name, PASSWORD).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(locked.error_code(), "too_many_attempts");
}

// ---------------------------------------------------------------------------
// Upgrades
// ---------------------------------------------------------------------------

/// A config for a hub in `dir` (data in `dir/state`).
fn hub_config(dir: &std::path::Path) -> (config::HubConfig, std::path::PathBuf) {
    let path = dir.join("blentinel_hub.toml");
    let settings = config::InstallSettings {
        host: "127.0.0.1".into(),
        port: 1,
        https_port: 2,
        state_dir: dir.join("state"),
    };
    std::fs::write(&path, config::install_config(&settings)).unwrap();
    let config = config::load_from(&path).unwrap();
    std::fs::create_dir_all(&config.server.state_dir).unwrap();
    (config, path)
}

/// Make `db` a database as the previous hub left it: every migration but the
/// newest. Returns its schema version.
async fn previous_hub_database(db: &std::path::Path) -> (sqlx::SqlitePool, i64) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/sqlite");
    let mut files: Vec<_> = std::fs::read_dir(&source)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    files.sort();
    files.pop(); // the newest
    let older = tempfile::tempdir().unwrap();
    for file in &files {
        std::fs::copy(source.join(file), older.path().join(file)).unwrap();
    }
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(older.path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    (pool, storage::latest_schema_version() - 1)
}

#[tokio::test]
async fn an_older_database_is_backed_up_before_it_is_upgraded() {
    let dir = tempfile::tempdir().unwrap();
    let (config, config_path) = hub_config(dir.path());
    let db = config.server.resolved_db_path();
    let (pool, previous) = previous_hub_database(&db).await;
    pool.close().await;

    let store = upgrade::open_database(&config, &config_path, chrono::Utc::now())
        .await
        .expect("upgraded");
    assert_eq!(
        store.schema_version().await.unwrap(),
        storage::latest_schema_version()
    );
    let made = backups::list(&config.backups_dir()).unwrap();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].kind, BackupKind::PreUpgrade);
    let manifest = backup::manifest_of(&config.backups_dir().join(&made[0].name)).unwrap();
    assert_eq!(
        manifest.schema_version, previous,
        "the database before the upgrade"
    );
}

#[tokio::test]
async fn a_new_or_current_database_gets_no_upgrade_backup() {
    let dir = tempfile::tempdir().unwrap();
    let (config, config_path) = hub_config(dir.path());
    for _ in 0..2 {
        // First: created. Second: already current.
        let store = upgrade::open_database(&config, &config_path, chrono::Utc::now())
            .await
            .unwrap();
        store.close().await;
    }
    assert!(backups::list(&config.backups_dir()).unwrap().is_empty());
}

/// SQL that gets in the way of the newest migration, so it fails half-way:
/// the table it creates, or the column it adds, already there.
fn in_the_way_of_newest_migration() -> String {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/sqlite");
    let mut files: Vec<_> = std::fs::read_dir(&source)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let sql = std::fs::read_to_string(files.last().unwrap()).unwrap();
    // The first statement that creates something, comments aside.
    let code: String = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let words: Vec<&str> = code.split_whitespace().collect();
    for (i, pair) in words.windows(2).enumerate() {
        match pair {
            ["CREATE", "TABLE"] => {
                return format!("CREATE TABLE {} (x INTEGER)", words[i + 2]);
            }
            ["ADD", "COLUMN"] => {
                // ALTER TABLE <table> ADD COLUMN <column> ...
                let table = words[i - 1];
                let column = words[i + 2];
                return format!("ALTER TABLE {table} ADD COLUMN {column} INTEGER");
            }
            _ => {}
        }
    }
    panic!("the newest migration creates no table and adds no column")
}

#[tokio::test]
async fn a_failed_upgrade_puts_the_database_back_and_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let (config, config_path) = hub_config(dir.path());
    let db = config.server.resolved_db_path();
    let (pool, previous) = previous_hub_database(&db).await;
    // Something in the way of the newest migration: it fails half-way.
    sqlx::query(&in_the_way_of_newest_migration())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let error = match upgrade::open_database(&config, &config_path, chrono::Utc::now()).await {
        Ok(_) => panic!("the upgrade can't succeed"),
        Err(e) => format!("{e:#}"),
    };
    assert!(error.contains("was put back"), "{error}");

    // The database is the one from before, untouched by the half-done upgrade.
    let store = storage::open_read_only(&db).await.unwrap();
    assert_eq!(store.schema_version().await.unwrap(), previous);
    store.close().await;
    let failed: Vec<_> = std::fs::read_dir(&config.server.state_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("failed-upgrade-"))
        .collect();
    assert_eq!(failed.len(), 1, "the failed database is kept");
    assert!(
        config
            .server
            .state_dir
            .join(&failed[0])
            .join("blentinel.db")
            .exists()
    );
}
