//! `hub status`: is the hub running, where, and in what state (D-52).
//! Reads only: the database is opened read-only.

use std::fmt::Write as _;
use std::path::Path;

use crate::config::HubConfig;
use crate::model::CompanyScope;
use crate::storage;

use super::{answers, platform, ports, setup_code, web_url};

/// The status report, ready to print.
pub async fn status(config: &HubConfig, config_path: &Path) -> String {
    let mut out = String::new();
    let line = |out: &mut String, label: &str, value: String| {
        let _ = writeln!(out, "  {label:<10} {value}");
    };
    let _ = writeln!(out, "Blentinel hub {}", env!("CARGO_PKG_VERSION"));
    line(&mut out, "Service", platform::service_state().to_string());
    line(&mut out, "Config", config_path.display().to_string());
    line(
        &mut out,
        "Data",
        config.server.state_dir.display().to_string(),
    );
    for (what, port) in ports(config) {
        let answering = if answers(&config.server.host, port) {
            "answering"
        } else {
            "not answering"
        };
        line(
            &mut out,
            "Listening",
            format!("{}:{port} ({what}): {answering}", config.server.host),
        );
    }
    line(&mut out, "Web UI", web_url(config));
    if let Some(code) = setup_code(&config.server.state_dir) {
        line(
            &mut out,
            "Setup",
            format!("waiting for the first Owner: setup code {code}"),
        );
    }
    line(
        &mut out,
        "Database",
        database(&config.server.resolved_db_path()).await,
    );
    if config.server.tls.enabled {
        let cert = config.server.resolve_path(&config.server.tls.cert_path);
        line(&mut out, "TLS cert", certificate(&cert));
    }
    out
}

/// Size, schema and what's in it.
async fn database(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return format!("{} (not created yet)", path.display());
    };
    // Recent writes may still be in the write-ahead log beside it.
    let wal = std::fs::metadata(format!("{}-wal", path.display())).map_or(0, |m| m.len());
    let size = format!("{:.1} MB", (meta.len() + wal) as f64 / 1_048_576.0);
    let store = match storage::open_read_only(path).await {
        Ok(store) => store,
        Err(e) => return format!("{size}, can't be read: {e:#}"),
    };
    let schema = store.schema_version().await.unwrap_or(0);
    let latest = storage::latest_schema_version();
    let probes = store
        .probe_records(&CompanyScope::All)
        .await
        .unwrap_or_default();
    let revoked = probes.iter().filter(|p| p.state != "active").count();
    let users = store.user_count().await.unwrap_or(0);
    let schema = if schema < latest {
        format!("schema {schema}, updated to {latest} at the next start")
    } else {
        format!("schema {schema}")
    };
    format!(
        "{size}, {schema}; {} probes ({revoked} revoked), {users} users",
        probes.len()
    )
}

/// When the certificate expires.
fn certificate(path: &Path) -> String {
    let Ok(pem) = std::fs::read(path) else {
        return format!("{} (created at the first start)", path.display());
    };
    match crate::tls::cert_not_after(&pem) {
        Ok(expires) => {
            let days = (expires - time::OffsetDateTime::now_utc()).whole_days();
            format!("expires {} ({days} days)", expires.date())
        }
        Err(e) => format!("can't be read: {e:#}"),
    }
}
