//! Opening the database: versioned migrations and per-connection settings.
#![cfg(feature = "ssr")]

use hub_lib::storage::SqliteStorage;

async fn scalar(pool: &sqlx::SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

#[tokio::test]
async fn fresh_database_is_migrated_and_reopening_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hub.db");

    let storage = SqliteStorage::open(&path).await.expect("first open");
    let pool = storage.pool();
    let applied = scalar(
        pool,
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1",
    )
    .await;
    pool.close().await;
    let storage = SqliteStorage::open(&path).await.expect("second open");
    let pool = storage.pool();

    assert!(applied >= 1);
    assert_eq!(
        scalar(
            pool,
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1"
        )
        .await,
        applied,
        "reopening must not re-apply migrations"
    );
}

#[tokio::test]
async fn database_from_before_migrations_is_refused_with_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let legacy = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE reports (id INTEGER PRIMARY KEY)")
        .execute(&legacy)
        .await
        .unwrap();
    legacy.close().await;

    let error = SqliteStorage::open(&path)
        .await
        .expect_err("legacy DB must be refused")
        .to_string();

    assert!(error.contains("delete the database file"), "{error}");
    assert!(error.contains("D-30"), "{error}");
}

#[tokio::test]
async fn every_pooled_connection_gets_wal_and_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::open(&dir.path().join("hub.db"))
        .await
        .unwrap();
    let pool = storage.pool();

    // Hold several connections at once so the pool can't hand back the same one.
    let mut connections = Vec::new();
    for _ in 0..3 {
        connections.push(pool.acquire().await.unwrap());
    }
    for conn in &mut connections {
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        assert_eq!(fk, 1);
        assert_eq!(mode.to_lowercase(), "wal");
    }
}

#[tokio::test]
async fn deleting_a_report_deletes_its_resource_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::open(&dir.path().join("hub.db"))
        .await
        .unwrap();
    let pool = storage.pool();
    sqlx::query(
        "INSERT INTO reports (id, probe_id, company_id, hostname, site, timestamp, interval_seconds)
         VALUES (1, 'p', 'c', 'h', 's', '2026-03-10 12:00:00+00:00', 60)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO resource_statuses (report_id, name, resource_type, target, status) VALUES (1, 'a', 'ping', '1', 'Up')")
        .execute(pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM reports WHERE id = 1")
        .execute(pool)
        .await
        .unwrap();

    assert_eq!(
        scalar(pool, "SELECT COUNT(*) FROM resource_statuses").await,
        0
    );
}

#[tokio::test]
async fn resource_status_for_a_missing_report_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::open(&dir.path().join("hub.db"))
        .await
        .unwrap();
    let pool = storage.pool();

    let result = sqlx::query(
        "INSERT INTO resource_statuses (report_id, name, resource_type, target, status) VALUES (999, 'a', 'ping', '1', 'Up')",
    )
    .execute(pool)
    .await;

    assert!(result.is_err(), "foreign key must be enforced");
}
