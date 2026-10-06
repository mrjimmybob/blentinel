//! Shared integration-test harness for the hub.
//!
//! [`TestHub`] builds the real `AppState` + API router (`hub_lib::routes::api_router`)
//! on a throwaway SQLite database in a temp directory, with a [`ManualClock`] so
//! tests control time. Requests go through the router in-process (no sockets).
//!
//! [`TestProbe`] plays the role of a probe: it owns an Ed25519 identity, builds
//! signed reports, and seals them with the **same** `common::crypto` code the
//! real probe uses.
//!
//! Tests write results by check name and spec ([`ping`], [`metric`]); each
//! gets a stable check ID ([`check_id`]), and the hub is given a matching
//! check record before a report is sent or seeded, as if an admin had
//! created it (checks are hub records, D-49).
#![allow(dead_code)] // Each test binary uses a different subset of the helpers.

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use chrono::{DateTime, Utc};
use common::checks::{CheckId, CheckResult, CheckSpec, Health, Metric};
use common::models::StatusReport;
use ed25519_dalek::SigningKey;
use http_body_util::BodyExt;
use hub_lib::model::Role;
use hub_lib::service::alerts::{AlertEmail, Notifier};
use hub_lib::service::users::{self, UserFields};
use hub_lib::state::Reloadable;
use hub_lib::storage::{SaveOutcome, SqliteStorage, Storage};
use hub_lib::{AppState, args::Args, clock::ManualClock, config, routes};
use leptos::prelude::LeptosOptions;
use rand::rngs::OsRng;
use serde::de::DeserializeOwned;
use tempfile::TempDir;
use tokio::sync::RwLock;
use tower::ServiceExt;
use x25519_dalek::{PublicKey, StaticSecret};

/// The Owner every test hub starts with (except [`TestHub::without_users`]).
pub const OWNER: &str = "owner";
/// Password of [`OWNER`] and of every user [`TestHub::login_as`] creates.
pub const PASSWORD: &str = "test-password-123";
/// Default probe expiry used by test hubs (matches the production default).
pub const PROBE_TIMEOUT_SECS: u64 = 300;

/// The release key test hubs trust (`AppState::release_keys`, D-55).
pub fn release_key() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}

/// A folder holding a release of `version` signed with `key`: a manifest
/// for `binaries` (target, bytes), its signature and the files.
pub fn signed_release_with(key: &SigningKey, version: &str, binaries: &[(&str, &[u8])]) -> TempDir {
    use common::release::{
        Artifact, MANIFEST_FILE, MANIFEST_FORMAT, Manifest, Platform, SIGNATURE_FILE,
    };
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().expect("temp dir");
    let mut artifacts = Vec::new();
    for (target, bytes) in binaries {
        let file = Platform::by_target(target)
            .expect("known target")
            .file_name();
        std::fs::write(dir.path().join(&file), bytes).expect("write binary");
        artifacts.push(Artifact {
            target: target.to_string(),
            file,
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
        });
    }
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        version: version.into(),
        released: default_start(),
        protocol: common::protocol::PROTOCOL_VERSION,
        artifacts,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
    std::fs::write(dir.path().join(MANIFEST_FILE), &bytes).expect("write manifest");
    std::fs::write(
        dir.path().join(SIGNATURE_FILE),
        common::release::sign_manifest(&bytes, key),
    )
    .expect("write signature");
    dir
}

/// [`signed_release_with`] the key test hubs trust.
pub fn signed_release(version: &str, binaries: &[(&str, &[u8])]) -> TempDir {
    signed_release_with(&release_key(), version, binaries)
}

/// The same release as a `.tar.gz`, as uploaded on the admin page.
pub fn release_archive(folder: &std::path::Path) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for entry in std::fs::read_dir(folder).expect("read release") {
        let entry = entry.expect("entry");
        tar.append_path_with_name(entry.path(), entry.file_name())
            .expect("append");
    }
    tar.into_inner().expect("tar").finish().expect("gzip")
}

/// Parse an RFC 3339 timestamp. Test-only convenience.
pub fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("valid RFC 3339 timestamp in test")
        .with_timezone(&Utc)
}

/// The fixed instant test hubs start at unless told otherwise.
pub fn default_start() -> DateTime<Utc> {
    at("2026-03-10T12:00:00Z")
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "response body is not the expected JSON ({e}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The `code` of an error response; panics unless the body is the hub's
    /// JSON error envelope (`common::protocol::ErrorBody`).
    pub fn error_code(&self) -> String {
        self.json::<common::protocol::ErrorBody>().error.code
    }
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

pub struct TestHub {
    pub state: AppState,
    /// Every alert the hub tried to send (nothing leaves the test).
    pub notifier: Arc<RecordingNotifier>,
    /// The same database as `state.storage`, concretely, for SQL inspection in tests.
    sqlite: Arc<SqliteStorage>,
    pub clock: Arc<ManualClock>,
    router: Router,
    dir: TempDir,
    /// Probes this hub knows (ID → name), enrolled in `acme` at the start.
    /// [`TestHub::send_report`] moves one to the company its report names, as
    /// an admin would by re-enrolling it: the hub, not the report, decides
    /// where a probe belongs (D-47).
    known_probes: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl TestHub {
    /// A hub at [`default_start`] that knows (has enrolled) the given probes.
    pub async fn new(probes: &[&TestProbe]) -> Self {
        Self::starting_at(default_start(), probes).await
    }

    /// A hub whose clock starts at `start` and knows the given probes.
    pub async fn starting_at(start: DateTime<Utc>, probes: &[&TestProbe]) -> Self {
        Self::build(start, probes, false, true).await
    }

    /// A hub with alerting enabled (recipient: [`ALERT_RECIPIENT`]).
    pub async fn with_alerts(probes: &[&TestProbe]) -> Self {
        Self::build(default_start(), probes, true, true).await
    }

    /// A freshly installed hub: no users, a setup code pending.
    pub async fn without_users() -> Self {
        Self::build(default_start(), &[], false, false).await
    }

    async fn build(
        start: DateTime<Utc>,
        probes: &[&TestProbe],
        alerts: bool,
        with_owner: bool,
    ) -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let config_path = dir.path().join("blentinel_hub.toml");
        std::fs::write(&config_path, config_toml(dir.path(), alerts)).expect("write test config");
        let cfg = config::load_from(&config_path).expect("test config is valid");

        // Same code path as production: connect + versioned migrations.
        let sqlite = Arc::new(
            SqliteStorage::open(&cfg.server.resolved_db_path())
                .await
                .expect("open and migrate test database"),
        );

        let clock = Arc::new(ManualClock::new(start));
        let notifier = Arc::new(RecordingNotifier::default());
        let state = AppState {
            leptos_options: LeptosOptions::builder().output_name("hub").build(),
            storage: Reloadable::new(sqlite.clone() as Arc<dyn Storage>),
            hub_secret: Reloadable::new(StaticSecret::random_from_rng(OsRng)),
            config: Arc::new(RwLock::new(cfg)),
            args: quiet_args(),
            setup: Arc::new(hub_lib::service::setup::SetupCode::default()),
            login_limiter: Arc::default(),
            clock: clock.clone(),
            notifier: notifier.clone(),
            listeners_changed: Arc::new(tokio::sync::Notify::new()),
            release_keys: Arc::new(vec![release_key().verifying_key().to_bytes()]),
            backup_downloads: Arc::default(),
            config_path: config_path.clone(),
        };
        let router = routes::api_router(&state).with_state(state.clone());

        let hub = Self {
            state,
            notifier,
            sqlite,
            clock,
            router,
            dir,
            known_probes: std::sync::Mutex::new(
                probes
                    .iter()
                    .map(|p| (p.id.clone(), p.name.clone()))
                    .collect(),
            ),
        };
        if with_owner {
            hub.seed_user(OWNER, Role::Owner).await;
        }
        for probe in probes {
            let site = format!("{}-site", probe.name);
            hub.place_probe(&probe.id, &probe.name, "acme", &site).await;
        }
        hub.state
            .setup
            .ensure(hub.store(), hub.dir())
            .await
            .expect("setup code");
        hub
    }

    /// The same database and state behind a new router: what a hub restart
    /// sees (sessions are in the database, so they survive it).
    pub fn restarted_router(&self) -> Router {
        routes::api_router(&self.state).with_state(self.state.clone())
    }

    /// Create a user with password [`PASSWORD`] in the hub's *current*
    /// database (a reload may have switched it).
    pub async fn seed_user(&self, username: &str, role: Role) {
        let fields = UserFields {
            display_name: String::new(),
            email: None,
            role,
            scope: hub_lib::model::CompanyScope::All,
        };
        users::create_user(
            self.state.storage.get().as_ref(),
            None,
            username,
            fields,
            Some(PASSWORD),
            self.now(),
        )
        .await
        .expect("seed user");
    }

    /// Raw SQLite pool, for tests that inspect tables directly.
    pub fn pool(&self) -> &sqlx::SqlitePool {
        self.sqlite.pool()
    }

    /// The hub's database as a trait object: the one it started with (a
    /// reload test that switches databases checks through the API instead).
    pub fn store(&self) -> &dyn Storage {
        self.sqlite.as_ref()
    }

    pub fn dir(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Add a release of `version` signed with [`release_key`], holding
    /// `binaries` (target, bytes), as `hub release add` would (D-55).
    pub fn add_release(&self, version: &str, binaries: &[(&str, &[u8])]) {
        let folder = signed_release(version, binaries);
        hub_lib::service::releases::add_from_dir(
            &self.dir().join(hub_lib::service::releases::RELEASES_DIR),
            folder.path(),
            &[release_key().verifying_key().to_bytes()],
        )
        .expect("add release");
    }

    // --- config reload (D-38) ------------------------------------------------

    pub fn config_path(&self) -> std::path::PathBuf {
        self.dir.path().join("blentinel_hub.toml")
    }

    /// Replace the config file and reload it, as the file watcher would.
    pub async fn reload_with(&self, toml: &str) -> anyhow::Result<()> {
        std::fs::write(self.config_path(), toml).expect("write config");
        hub_lib::hot_reload::reload(&self.state, &self.config_path()).await
    }

    /// The current config file with `from` replaced by `to` (asserts it's there).
    pub fn edited_config(&self, from: &str, to: &str) -> String {
        let toml = std::fs::read_to_string(self.config_path()).expect("read config");
        assert!(
            toml.contains(from),
            "config has no {from:?}:
{toml}"
        );
        toml.replacen(from, to, 1)
    }

    pub fn now(&self) -> DateTime<Utc> {
        use hub_lib::clock::Clock;
        self.clock.now()
    }

    pub fn public_key(&self) -> [u8; 32] {
        *PublicKey::from(&self.state.hub_secret.get()).as_bytes()
    }

    /// The public half of the hub's signing key (it signs report acks).
    pub fn signing_key(&self) -> [u8; 32] {
        common::crypto::hub_signing_key(&self.state.hub_secret.get())
            .verifying_key()
            .to_bytes()
    }

    // --- raw requests -------------------------------------------------------

    pub async fn send(&self, request: Request<Body>) -> TestResponse {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router is infallible");
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        TestResponse {
            status,
            headers,
            body,
        }
    }

    pub async fn get(&self, path: &str, cookie: Option<&str>) -> TestResponse {
        self.send(build(Method::GET, path, cookie, None, Body::empty()))
            .await
    }

    /// POST JSON as the web UI does (with the CSRF header, D-43).
    pub async fn post_json(
        &self,
        path: &str,
        cookie: Option<&str>,
        json: serde_json::Value,
    ) -> TestResponse {
        let mut request = build(
            Method::POST,
            path,
            cookie,
            Some("application/json"),
            Body::from(json.to_string()),
        );
        request
            .headers_mut()
            .insert("x-blentinel", "1".parse().expect("header value"));
        self.send(request).await
    }

    /// POST JSON without the CSRF header (as a cross-site form would).
    pub async fn post_json_without_csrf(
        &self,
        path: &str,
        cookie: Option<&str>,
        json: serde_json::Value,
    ) -> TestResponse {
        let body = Body::from(json.to_string());
        self.send(build(
            Method::POST,
            path,
            cookie,
            Some("application/json"),
            body,
        ))
        .await
    }

    /// POST a file as the web UI uploads one (raw body, CSRF header).
    pub async fn post_file(&self, path: &str, cookie: Option<&str>, body: Vec<u8>) -> TestResponse {
        let mut request = build(
            Method::POST,
            path,
            cookie,
            Some("application/gzip"),
            Body::from(body),
        );
        request
            .headers_mut()
            .insert("x-blentinel", "1".parse().expect("header value"));
        self.send(request).await
    }

    pub async fn post_bytes(&self, path: &str, body: Vec<u8>) -> TestResponse {
        self.send(build(
            Method::POST,
            path,
            None,
            Some("application/octet-stream"),
            Body::from(body),
        ))
        .await
    }

    // --- sessions -----------------------------------------------------------

    /// POST `/api/auth/login`.
    pub async fn try_login(&self, username: &str, password: &str) -> TestResponse {
        let body = serde_json::json!({ "username": username, "password": password });
        self.post_json("/api/auth/login", None, body).await
    }

    /// Log in as `username` (password [`PASSWORD`]) and return the
    /// `name=value` cookie pair.
    pub async fn login_user(&self, username: &str) -> String {
        let response = self.try_login(username, PASSWORD).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "login as {username}: {}",
            response.text()
        );
        session_cookie(&response.headers).expect("login sets a session cookie")
    }

    /// A fresh session for an existing user, created directly in storage (no
    /// password hashing: for tests that need many sessions).
    pub async fn session_for(&self, username: &str) -> String {
        let store = self.state.storage.get();
        let user = store
            .user_by_username(username)
            .await
            .expect("look up user")
            .expect("user exists");
        let token = users::random_string(32);
        store
            .create_session(
                &users::session_hash(&token),
                &user.info.id,
                self.now(),
                self.now() + users::ABSOLUTE_TIMEOUT,
            )
            .await
            .expect("create session");
        format!("blentinel_session={token}")
    }

    /// Log in as the Owner.
    pub async fn login(&self) -> String {
        self.login_user(OWNER).await
    }

    /// Log in as a user with `role` (created on first use, named after it).
    pub async fn login_as(&self, role: Role) -> String {
        let username = role.as_str();
        let exists = self
            .state
            .storage
            .get()
            .user_by_username(username)
            .await
            .expect("look up user")
            .is_some();
        if !exists {
            self.seed_user(username, role).await;
        }
        self.login_user(username).await
    }

    /// GET an authenticated endpoint and decode its JSON (asserts 200).
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> T {
        let cookie = self.login().await;
        let response = self.get(path, Some(&cookie)).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "GET {path}: {}",
            response.text()
        );
        response.json()
    }

    // --- probe traffic ------------------------------------------------------

    /// Make the probe an enrolled, active probe of `company` / `site` (both
    /// created if needed), as "Add probe" + `probe enroll` would.
    pub async fn place_probe(&self, probe_id: &str, name: &str, company: &str, site: &str) {
        let store = self.state.storage.get();
        store
            .create_company(company, company, self.now())
            .await
            .expect("company");
        store
            .create_site(company, site, site, self.now())
            .await
            .expect("site");
        let current = store.probe_record(probe_id).await.expect("probe record");
        if current
            .as_ref()
            .is_some_and(|r| r.company_id == company && r.site_id == site && r.is_active())
        {
            return;
        }
        let record = hub_lib::storage::ProbeRecord {
            id: probe_id.to_string(),
            name: name.to_string(),
            company_id: company.to_string(),
            site_id: site.to_string(),
            state: hub_lib::storage::PROBE_ACTIVE.to_string(),
            hostname: None,
            os: None,
            arch: None,
            probe_version: None,
            enrolled_at: self.now(),
            token_id: None,
            interval_secs: common::checks::DEFAULT_INTERVAL_SECS,
            config_revision: None,
            config_rejected: None,
            update_failed: None,
        };
        store
            .save_probe_record(&record)
            .await
            .expect("save probe record");
    }

    /// Seal `report` for this hub and POST it to `/api/report`. A probe this
    /// hub knows is first placed in the report's company and site.
    pub async fn send_report(&self, report: &StatusReport) -> StatusCode {
        self.deliver_report(report).await.0.status
    }

    /// Send `report` from `probe` and open the hub's answer as the probe
    /// would: sealed for it, signed by the hub, bound to this report.
    pub async fn send_report_for_ack(
        &self,
        probe: &TestProbe,
        report: &StatusReport,
    ) -> (
        StatusCode,
        Result<common::ack::ReportAck, common::ack::AckError>,
    ) {
        let (response, sealed) = self.deliver_report(report).await;
        let ack = common::ack::open_ack(&response.body, &probe.key, &self.signing_key(), &sealed);
        (response.status, ack)
    }

    /// Seal and post `report` (placing a known probe first, and giving it the
    /// report's checks); the response and the sealed bytes sent.
    async fn deliver_report(&self, report: &StatusReport) -> (TestResponse, Vec<u8>) {
        let known = self
            .known_probes
            .lock()
            .expect("known probes")
            .get(&report.probe_id)
            .cloned();
        if let Some(name) = known {
            self.place_probe(&report.probe_id, &name, &report.company_id, &report.site)
                .await;
        }
        self.ensure_checks(report).await;
        let sealed = seal(report, &self.public_key());
        (self.post_bytes("/api/report", sealed.clone()).await, sealed)
    }

    /// Create the check behind each of the report's results that the hub
    /// doesn't have yet, if the probe has a record (checks belong to one).
    pub async fn ensure_checks(&self, report: &StatusReport) {
        let store = self.state.storage.get();
        if store
            .probe_record(&report.probe_id)
            .await
            .expect("probe record")
            .is_none()
        {
            return;
        }
        for result in &report.results {
            if store
                .check(&result.check_id)
                .await
                .expect("check")
                .is_some()
            {
                continue;
            }
            let Some((name, spec)) = registered_check(&result.check_id) else {
                continue; // made up by the test on purpose
            };
            let check = hub_lib::storage::CheckRecord {
                id: result.check_id.clone(),
                probe_id: report.probe_id.clone(),
                company_id: report.company_id.clone(),
                name,
                spec,
                enabled: true,
                created_at: self.now(),
                updated_at: self.now(),
                updated_by: None,
            };
            store.create_check(&check).await.expect("create check");
        }
    }

    /// Give an enrolled `probe` a check called `name`, as an admin would;
    /// returns its ID ([`check_id`]).
    pub async fn add_check(&self, probe: &TestProbe, name: &str, spec: CheckSpec) -> String {
        let result = Res {
            name: name.to_string(),
            spec,
            status: Health::Up,
            message: String::new(),
            latency_ms: None,
            metrics: vec![],
        }
        .result_for(probe);
        let record = self
            .store()
            .probe_record(&probe.id)
            .await
            .expect("probe record")
            .expect("the probe is enrolled");
        let mut report = probe.unsigned_report(&record.company_id, self.now(), vec![]);
        report.results.push(result);
        self.ensure_checks(&report).await;
        check_id(&probe.id, name)
    }

    /// Send a signed report from `probe` with the given results, timestamped
    /// at the hub's current time. Asserts the hub accepts it.
    pub async fn report_now(&self, probe: &TestProbe, company: &str, resources: Vec<Res>) {
        let report = probe.signed_report(company, self.now(), resources);
        assert_eq!(
            self.send_report(&report).await,
            StatusCode::OK,
            "hub must accept the report"
        );
    }

    /// Write a report straight into the database, bypassing ingestion.
    ///
    /// For seeding *history* (e.g. month-old data for uptime or archive tests):
    /// the ingest endpoint correctly refuses reports older than any probe buffers.
    pub async fn seed_report(&self, report: &StatusReport) {
        self.ensure_checks(report).await;
        let outcome = self.store().save_report(report).await.expect("seed report");
        assert_eq!(
            outcome,
            SaveOutcome::Stored,
            "seed reports in increasing timestamp order"
        );
    }

    /// Run the expiry sweep the hub's background task performs every 10 s.
    pub async fn sweep_expired(&self) -> Vec<(String, String)> {
        self.store()
            .expire_silent_probes(PROBE_TIMEOUT_SECS, self.now())
            .await
            .expect("expiry sweep")
    }
}

fn config_toml(dir: &std::path::Path, alerts: bool) -> String {
    // TOML literal strings ('...') take Windows backslashes verbatim.
    let mut toml = format!(
        "[server]\n\
         host = '127.0.0.1'\n\
         port = 3000\n\
         state_dir = '{dir}'\n\
         db_path = 'blentinel.db'\n\
         probe_timeout_secs = {PROBE_TIMEOUT_SECS}\n\
         \n\
         [server.tls]\n\
         enabled = false\n\
         \n\
         [retention]\n\
         archive_path = '{archives}'\n",
        dir = dir.display(),
        archives = dir.join("archives").display(),
    );
    if alerts {
        toml.push_str(&format!(
            // The SMTP server is never contacted: tests use RecordingNotifier.
            "\n[alerts]\nenabled = true\ndefault_recipients = ['{ALERT_RECIPIENT}']\n\
             \n[alerts.smtp]\nserver = 'smtp.invalid'\nfrom = 'hub@example.com'\n"
        ));
    }
    toml
}

fn quiet_args() -> Args {
    Args {
        verbose: false,
        debug: false,
        init: false,
        print_public_key: false,
        user: None,
        ops: None,
        config_path: None,
    }
}

fn build(
    method: Method,
    path: &str,
    cookie: Option<&str>,
    content_type: Option<&str>,
    body: Body,
) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if let Some(content_type) = content_type {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    request.body(body).expect("valid request")
}

/// Extract `blentinel_session=<id>` from a `Set-Cookie` header.
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let set_cookie = headers.get(header::SET_COOKIE)?.to_str().ok()?;
    let pair = set_cookie.split(';').next()?.trim();
    pair.starts_with("blentinel_session=")
        .then(|| pair.to_string())
}

/// Seal any serialisable value for a hub, exactly as the probe does.
pub fn seal<T: serde::Serialize>(value: &T, hub_public_key: &[u8; 32]) -> Vec<u8> {
    let bytes = serde_json::to_vec(value).expect("serialise");
    common::crypto::seal_for_hub(&bytes, hub_public_key).expect("seal")
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

pub struct TestProbe {
    pub name: String,
    /// Hex of the Ed25519 verifying key, which is also the probe ID.
    pub id: String,
    key: SigningKey,
}

impl TestProbe {
    pub fn new(name: &str) -> Self {
        let key = SigningKey::generate(&mut OsRng);
        let id = hex::encode(key.verifying_key().as_bytes());
        Self {
            name: name.to_string(),
            id,
            key,
        }
    }

    /// An unsigned report (hostname and site derived from the probe name).
    pub fn unsigned_report(
        &self,
        company: &str,
        timestamp: DateTime<Utc>,
        resources: Vec<Res>,
    ) -> StatusReport {
        StatusReport {
            protocol_version: common::protocol::PROTOCOL_VERSION,
            probe_id: self.id.clone(),
            company_id: company.to_string(),
            hostname: format!("{}-host", self.name),
            site: format!("{}-site", self.name),
            timestamp,
            interval_seconds: 60,
            results: resources.into_iter().map(|r| r.result_for(self)).collect(),
            config_revision: None,
            config_rejected: None,
            probe_version: None,
            update_failed: None,
            command_results: vec![],
            signature: None,
        }
    }

    /// A report signed exactly as the real probe signs it (`to_signable()` → JSON → Ed25519).
    pub fn signed_report(
        &self,
        company: &str,
        timestamp: DateTime<Utc>,
        resources: Vec<Res>,
    ) -> StatusReport {
        let mut report = self.unsigned_report(company, timestamp, resources);
        self.sign(&mut report);
        report
    }

    /// Sign any bytes with this probe's key (e.g. an enrollment request).
    pub fn sign_bytes(&self, data: &[u8]) -> Vec<u8> {
        common::crypto::sign(data, &self.key)
    }

    pub fn sign(&self, report: &mut StatusReport) {
        let signable = serde_json::to_vec(&report.to_signable()).expect("serialise signable");
        report.signature = Some(common::crypto::sign(&signable, &self.key));
    }

    /// The probe's identity key (to open the hub's answers).
    pub fn key(&self) -> &SigningKey {
        &self.key
    }
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// A check's result as a test writes it: the check (by name and spec) and
/// what it found. [`TestProbe::unsigned_report`] turns it into a
/// `CheckResult` for the check's ID ([`check_id`]).
#[derive(Debug, Clone)]
pub struct Res {
    pub name: String,
    pub spec: CheckSpec,
    pub status: Health,
    pub message: String,
    pub latency_ms: Option<u64>,
    pub metrics: Vec<Metric>,
}

impl Res {
    fn result_for(self, probe: &TestProbe) -> CheckResult {
        let id = CheckId(check_id(&probe.id, &self.name));
        CHECKS
            .lock()
            .expect("check registry")
            .insert(id.clone(), (self.name, self.spec));
        CheckResult {
            check_id: id,
            status: self.status,
            message: self.message,
            latency_ms: self.latency_ms,
            metrics: self.metrics,
        }
    }
}

/// Every check a test has reported on, by ID: what [`TestHub::ensure_checks`] creates.
static CHECKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<CheckId, (String, CheckSpec)>>,
> = std::sync::LazyLock::new(Default::default);

fn registered_check(id: &CheckId) -> Option<(String, CheckSpec)> {
    CHECKS.lock().expect("check registry").get(id).cloned()
}

/// The ID of the check called `name` of the probe `probe_id` (stable, so
/// tests can name it).
pub fn check_id(probe_id: &str, name: &str) -> String {
    format!("{}.{name}", &probe_id[..16])
}

/// A ping check that is up (with latency) or down.
pub fn ping(name: &str, target: &str, up: bool) -> Res {
    Res {
        name: name.to_string(),
        spec: CheckSpec::Ping {
            host: target.to_string(),
        },
        status: if up { Health::Up } else { Health::Down },
        message: if up {
            "Responded".into()
        } else {
            "Request timed out".into()
        },
        latency_ms: up.then_some(12),
        metrics: vec![],
    }
}

/// A check of the probe's machine (CPU/memory/disk style) with its usage,
/// always reported as Up.
pub fn metric(name: &str, spec: CheckSpec, value: f64, unit: &str) -> Res {
    Res {
        name: name.to_string(),
        spec,
        status: Health::Up,
        message: format!("{name}: {value:.1}{unit}"),
        latency_ms: None,
        metrics: vec![Metric::new(common::checks::metric::USAGE, value, unit)],
    }
}

// ---------------------------------------------------------------------------
// Alerts
// ---------------------------------------------------------------------------

/// Recipient configured on hubs built with [`TestHub::with_alerts`].
pub const ALERT_RECIPIENT: &str = "ops@example.com";

/// A notifier that records alerts instead of sending them.
#[derive(Default)]
pub struct RecordingNotifier {
    sent: std::sync::Mutex<Vec<AlertEmail>>,
}

impl RecordingNotifier {
    /// Subjects of every alert sent so far, in order.
    pub fn subjects(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.subject.clone())
            .collect()
    }

    pub fn sent(&self) -> Vec<AlertEmail> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Notifier for RecordingNotifier {
    async fn send(&self, email: AlertEmail) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(email);
        Ok(())
    }
}

impl TestHub {
    /// Wait (up to 2 s) until at least `n` alerts were sent by background tasks.
    pub async fn wait_for_alerts(&self, n: usize) -> Vec<String> {
        for _ in 0..200 {
            if self.notifier.subjects().len() >= n {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        self.notifier.subjects()
    }
}
