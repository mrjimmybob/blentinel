//! Hub upgrades (M4-04, D-75): the opt-in check for a newer release on the
//! admin page, and preparing an upgrade from a signed release (download,
//! signature, checksums). Switching and rolling back are unit-tested in
//! `ops::hub_upgrade` with a fake service.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use common::hub_release::{
    HubRelease, PROBES_ARCHIVE, RELEASE_FILE, RELEASE_FORMAT, SUMS_FILE, SUMS_SIGNATURE_FILE,
    current_target, hub_file,
};
use ed25519_dalek::SigningKey;
use hub_lib::model::{HubUpgradeInfo, Role};
use hub_lib::ops::hub_upgrade::prepare;
use hub_lib::service::hub_upgrades::{self, Source};
use sha2::{Digest, Sha256};
use support::*;

const LINUX: &str = "x86_64-unknown-linux-musl";

/// A hub release of `version` in a folder, as `blentinelmake release hub`
/// writes it, signed with `key`: this machine's hub (`hub`), the probes,
/// `release.json`, and the signed `SHA256SUMS`.
fn hub_release_with(key: &SigningKey, version: &str, hub: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(dir.path().join(name), bytes).unwrap();
    write(&hub_file(current_target().unwrap()), hub);
    let probes = signed_release(version, &[(LINUX, b"probe")]);
    write(PROBES_ARCHIVE, &release_archive(probes.path()));
    let info = HubRelease {
        format: RELEASE_FORMAT,
        version: version.into(),
        released: "2026-10-09T10:00:00Z".parse().unwrap(),
        commit: "c0".repeat(20),
    };
    write(RELEASE_FILE, &serde_json::to_vec(&info).unwrap());
    let mut sums = String::new();
    let mut names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let bytes = std::fs::read(dir.path().join(&name)).unwrap();
        sums.push_str(&format!(
            "{}  {name}\n",
            hex::encode(Sha256::digest(&bytes))
        ));
    }
    write(SUMS_FILE, sums.as_bytes());
    write(
        SUMS_SIGNATURE_FILE,
        &common::crypto::sign(sums.as_bytes(), key),
    );
    dir
}

fn hub_release(version: &str) -> tempfile::TempDir {
    hub_release_with(&release_key(), version, b"the new hub")
}

async fn point_at(hub: &TestHub, folder: &std::path::Path) {
    hub.state.config.write().await.upgrades.release_url = folder.display().to_string();
}

async fn info(hub: &TestHub) -> HubUpgradeInfo {
    hub.get_json("/api/admin/hub-upgrade").await
}

#[tokio::test]
async fn the_check_is_off_until_an_admin_turns_it_on() {
    let hub = TestHub::new(&[]).await;
    let release = hub_release("999.0.0");
    point_at(&hub, release.path()).await;

    let shown = info(&hub).await;
    assert_eq!(shown.running, env!("CARGO_PKG_VERSION"));
    assert!(!shown.check_enabled);
    assert_eq!((shown.checked_at, shown.available), (None, None));
    assert!(shown.command.ends_with("upgrade"), "{}", shown.command);

    // Off: the background sweep doesn't look.
    let url = release.path().display().to_string();
    let keys = hub.state.trusted_release_keys().await;
    hub_upgrades::sweep(hub.store(), &url, &keys, hub.now())
        .await
        .unwrap();
    assert_eq!(info(&hub).await.checked_at, None);

    let owner = hub.login().await;
    let on = hub
        .post_json(
            "/api/admin/hub-upgrade",
            Some(&owner),
            serde_json::json!({ "enabled": true }),
        )
        .await;
    assert_eq!(on.status, StatusCode::OK);
    assert!(on.json::<HubUpgradeInfo>().check_enabled);
    hub_upgrades::sweep(hub.store(), &url, &keys, hub.now())
        .await
        .unwrap();
    let shown = info(&hub).await;
    assert_eq!(shown.available.as_deref(), Some("999.0.0"));
    assert!(shown.checked_at.is_some() && shown.error.is_none());

    // Once a day: not again an hour later.
    std::fs::remove_dir_all(release.path()).unwrap();
    hub.clock.advance(chrono::Duration::hours(1));
    hub_upgrades::sweep(hub.store(), &url, &keys, hub.now())
        .await
        .unwrap();
    assert!(info(&hub).await.error.is_none(), "not checked again yet");
    hub.clock.advance(chrono::Duration::hours(23));
    hub_upgrades::sweep(hub.store(), &url, &keys, hub.now())
        .await
        .unwrap();
    let shown = info(&hub).await;
    assert!(
        shown.error.is_some(),
        "a day later it looked, and found nothing"
    );
    assert_eq!(
        shown.available.as_deref(),
        Some("999.0.0"),
        "what it found before stays"
    );

    let actions: Vec<String> = hub
        .get_json::<Vec<serde_json::Value>>("/api/audit?action=hub")
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        actions.contains(&"hub.upgrade_check".to_string()),
        "{actions:?}"
    );
}

#[tokio::test]
async fn check_now_shows_a_newer_release_and_refuses_an_unsigned_one() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let check = || {
        hub.post_json(
            "/api/admin/hub-upgrade/check",
            Some(&owner),
            serde_json::json!({}),
        )
    };

    let same = hub_release(env!("CARGO_PKG_VERSION"));
    point_at(&hub, same.path()).await;
    let shown: HubUpgradeInfo = check().await.json();
    assert_eq!(shown.latest.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(shown.available, None, "up to date");

    let stranger = SigningKey::from_bytes(&[5; 32]);
    let forged = hub_release_with(&stranger, "999.0.0", b"hub");
    point_at(&hub, forged.path()).await;
    let shown: HubUpgradeInfo = check().await.json();
    assert_eq!(shown.available, None);
    assert!(
        shown
            .error
            .as_deref()
            .is_some_and(|e| e.contains("isn't signed")),
        "{:?}",
        shown.error
    );
}

#[tokio::test]
async fn only_admins_change_the_check() {
    let hub = TestHub::new(&[]).await;
    let technician = hub.login_as(Role::Technician).await;
    for path in ["/api/admin/hub-upgrade", "/api/admin/hub-upgrade/check"] {
        let refused = hub
            .post_json(
                path,
                Some(&technician),
                serde_json::json!({ "enabled": true }),
            )
            .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn an_upgrade_stages_only_a_newer_signed_hub_that_matches_its_checksum() {
    let program = tempfile::tempdir().unwrap();
    let exe = program.path().join("hub");
    let keys = [release_key().verifying_key().to_bytes()];

    let release = hub_release("999.0.0");
    let prepared = prepare(
        &Source::parse(&release.path().display().to_string()),
        &keys,
        &exe,
    )
    .await
    .unwrap();
    assert_eq!(prepared.version, "999.0.0");
    assert_eq!(std::fs::read(&prepared.staged).unwrap(), b"the new hub");
    assert_eq!(prepared.staged, exe.with_extension("new"));
    let probes = prepared.probes.expect("the release's probes");
    let added = hub_lib::service::releases::add_from_archive(
        &program.path().join("releases"),
        &probes,
        &keys,
    )
    .unwrap();
    assert_eq!(added.version, "999.0.0");

    // Not newer.
    let old = hub_release("0.0.1");
    let error = prepare(
        &Source::parse(&old.path().display().to_string()),
        &keys,
        &exe,
    )
    .await
    .err()
    .unwrap();
    assert!(
        error.to_string().contains("nothing to upgrade"),
        "{error:#}"
    );

    // A hub that isn't the one SHA256SUMS signs.
    let tampered = hub_release("999.0.0");
    std::fs::write(
        tampered.path().join(hub_file(current_target().unwrap())),
        b"something else",
    )
    .unwrap();
    let error = prepare(
        &Source::parse(&tampered.path().display().to_string()),
        &keys,
        &exe,
    )
    .await
    .err()
    .unwrap();
    assert!(
        format!("{error:#}").contains("isn't the file the release signs"),
        "{error:#}"
    );

    // Signed by a key this hub doesn't trust.
    let forged = hub_release_with(&SigningKey::from_bytes(&[5; 32]), "999.0.0", b"hub");
    assert!(
        prepare(
            &Source::parse(&forged.path().display().to_string()),
            &keys,
            &exe
        )
        .await
        .is_err()
    );
}

/// 0013 §6.2, D-106: `hub upgrade` takes a staging-signed hub only on a hub
/// whose config says `[upgrades] staging = true`.
#[tokio::test]
async fn an_upgrade_takes_a_staging_signed_hub_only_on_a_staging_hub() {
    let hub = TestHub::new(&[]).await;
    let program = tempfile::tempdir().unwrap();
    let exe = program.path().join("hub");
    let release = hub_release_with(&staging_key(), "999.0.0-dev.10", b"the dev hub");
    let source = Source::parse(&release.path().display().to_string());

    let production = hub.state.trusted_release_keys().await;
    let error = prepare(&source, &production, &exe).await.err().unwrap();
    assert!(
        format!("{error:#}").contains("SHA256SUMS"),
        "refused for its signature: {error:#}"
    );

    hub.state.config.write().await.upgrades.staging = true;
    let staging = hub.state.trusted_release_keys().await;
    let prepared = prepare(&source, &staging, &exe).await.unwrap();
    assert_eq!(prepared.version, "999.0.0-dev.10");
}

#[tokio::test]
async fn the_self_test_upgrades_a_copy_of_the_database_and_leaves_the_real_one() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("blentinel_hub.toml");
    std::fs::write(
        &config_path,
        format!(
            "[server]\nhost = '127.0.0.1'\nport = 1\nstate_dir = '{}'\ndb_path = 'hub.db'\n",
            dir.path().display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let config = hub_lib::config::load_from(&config_path).unwrap();

    // No key yet: refused, without creating one.
    let error = hub_lib::ops::self_test::run(&config_path)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("key"), "{error:#}");
    let key = config.server.resolved_identity_key_path();
    assert!(!key.exists());

    hub_lib::identity::load_or_create_hub_key(&key).unwrap();
    // No database yet: nothing to upgrade.
    hub_lib::ops::self_test::run(&config_path).await.unwrap();

    let db = config.server.resolved_db_path();
    let store = hub_lib::storage::open(&db).await.unwrap();
    store.close().await;
    let before = std::fs::read(&db).unwrap();
    hub_lib::ops::self_test::run(&config_path).await.unwrap();
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "the real database is untouched"
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".self-test-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
