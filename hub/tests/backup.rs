//! `hub backup` and `hub restore` (M2-08, D-52): a backup made from a live
//! hub restores into another data folder with its users, probes and key.
#![cfg(feature = "ssr")]

mod support;

use hub_lib::config;
use hub_lib::ops::backup;
use hub_lib::storage;
use support::*;

/// A config for a new, empty hub in `dir`.
fn fresh_config(dir: &std::path::Path) -> (config::HubConfig, std::path::PathBuf) {
    let path = dir.join("blentinel_hub.toml");
    let settings = config::InstallSettings {
        host: "127.0.0.1".into(),
        port: 1,
        https_port: 2,
        state_dir: dir.join("state"),
        staging: false,
    };
    std::fs::write(&path, config::install_config(&settings)).unwrap();
    (config::load_from(&path).unwrap(), path)
}

#[tokio::test]
async fn a_backup_restores_users_probes_and_the_identity_key_elsewhere() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("gw", "10.0.0.1", true)])
        .await;
    let source = config::load_from(&hub.config_path()).unwrap();
    let key = source.server.resolved_identity_key_path();
    std::fs::write(&key, [9u8; 32]).unwrap();
    // The ACME account (D-62): a restored hub keeps it.
    let account = hub_lib::keys::KeyFiles::of(&source).acme_account;
    std::fs::write(&account, b"{}").unwrap();
    // The data key (D-66): the database's secrets are useless without it.
    std::fs::write(hub_lib::keys::KeyFiles::of(&source).data_key, [5u8; 32]).unwrap();

    let out = hub.dir().join("backups").join("hub.tar.gz");
    let manifest = backup::backup(hub.store(), &source, &hub.config_path(), &out, hub.now())
        .await
        .expect("backup while the hub has the database open");
    assert_eq!(manifest.schema_version, storage::latest_schema_version());
    assert_eq!(backup::manifest_of(&out).unwrap(), manifest);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "it holds the private key");
    }

    let target = tempfile::tempdir().unwrap();
    let (config, config_path) = fresh_config(target.path());
    let aside = backup::restore(&out, &config, &config_path, false, hub.now()).unwrap();
    assert_eq!(aside, None, "nothing was there to move aside");

    let store = storage::open(&config.server.resolved_db_path())
        .await
        .unwrap();
    assert!(store.user_by_username(OWNER).await.unwrap().is_some());
    assert!(store.probe_record(&probe.id).await.unwrap().is_some());
    let key = config.server.resolved_identity_key_path();
    assert_eq!(std::fs::read(&key).unwrap(), [9u8; 32]);
    let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([9u8; 32]));
    assert_eq!(
        std::fs::read_to_string(key.with_extension("pub")).unwrap(),
        hex::encode(public.as_bytes()),
        "the .pub matches the restored key"
    );
    assert_eq!(
        std::fs::read(hub_lib::keys::KeyFiles::of(&config).acme_account).unwrap(),
        b"{}"
    );
    assert_eq!(
        std::fs::read(hub_lib::keys::KeyFiles::of(&config).data_key).unwrap(),
        [5u8; 32]
    );
    assert!(
        std::path::PathBuf::from(format!("{}.from-backup", config_path.display())).exists(),
        "the backed-up config is kept for reference, not over the current one"
    );
}

#[tokio::test]
async fn restoring_moves_the_current_files_aside_and_refuses_a_running_hub() {
    let hub = TestHub::new(&[]).await;
    let source = config::load_from(&hub.config_path()).unwrap();
    let out = hub.dir().join("b.tar.gz");
    backup::backup(hub.store(), &source, &hub.config_path(), &out, hub.now())
        .await
        .unwrap();

    let target = tempfile::tempdir().unwrap();
    let (config, config_path) = fresh_config(target.path());
    std::fs::create_dir_all(&config.server.state_dir).unwrap();
    // Files already there (as with the hub stopped).
    std::fs::write(config.server.resolved_db_path(), b"the old database").unwrap();

    let running = backup::restore(&out, &config, &config_path, true, hub.now());
    assert!(running.unwrap_err().to_string().contains("running"));

    let aside = backup::restore(&out, &config, &config_path, false, hub.now())
        .unwrap()
        .expect("the old database was moved aside");
    assert_eq!(
        std::fs::read(aside.join("blentinel.db")).unwrap(),
        b"the old database"
    );
    assert!(aside.starts_with(&config.server.state_dir));
}

#[tokio::test]
async fn a_backup_from_a_newer_hub_or_another_file_is_refused() {
    let target = tempfile::tempdir().unwrap();
    let (config, config_path) = fresh_config(target.path());

    let not_a_backup = target.path().join("notes.txt");
    std::fs::write(&not_a_backup, "hello").unwrap();
    assert!(
        backup::restore(
            &not_a_backup,
            &config,
            &config_path,
            false,
            chrono::Utc::now()
        )
        .is_err()
    );

    // A manifest from the future.
    let future = target.path().join("future.tar.gz");
    let manifest = serde_json::json!({
        "format": 1, "hub_version": "9.0.0", "created_at": "2030-01-01T00:00:00Z",
        "schema_version": storage::latest_schema_version() + 1,
    })
    .to_string();
    let file = std::fs::File::create(&future).unwrap();
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::default(),
    ));
    for (name, bytes) in [
        ("manifest.json", manifest.as_bytes()),
        ("blentinel.db", b"x"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        archive.append_data(&mut header, name, bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();

    let refused = backup::restore(&future, &config, &config_path, false, chrono::Utc::now());
    assert!(refused.unwrap_err().to_string().contains("newer hub"));
    assert!(
        !config.server.resolved_db_path().exists(),
        "nothing was written"
    );
}
