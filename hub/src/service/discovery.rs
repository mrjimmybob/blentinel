//! What a probe found on its network, and the checks that suggests (M5-18,
//! proposal 0006 §4, D-101).
//!
//! A sweep is the one thing a probe does to devices nobody named, and some
//! networks' owners forbid it. So it is off for every company until someone
//! who may manage the hub's data allows it for that company; then whoever
//! may send a probe commands asks for one sweep at a time. The hub keeps a
//! probe's latest sweep and proposes checks for the devices in it. Nothing
//! is checked until someone adds it.

use common::checks::{CheckSpec, DbEngine, LdapSecurity};
use common::discovery::{Device, Discovery, port_label};

use crate::model::{CompanyDetail, CompanyScope, FoundDevice, NetworkInfo, Suggestion};
use crate::service::companies::{CompanyError, CompanyResult};
use crate::service::probes::{ProbeResult, scoped_record};
use crate::storage::Storage;

/// Why a sweep can't be asked for, for people.
pub const NOT_ALLOWED: &str =
    "network discovery isn't allowed for this company: an Admin allows it on the company's page";

/// Allow or forbid network discovery for a company.
pub async fn set_allowed(
    store: &dyn Storage,
    company_id: &str,
    allowed: bool,
) -> CompanyResult<CompanyDetail> {
    if !store.set_company_discovery(company_id, allowed).await? {
        return Err(CompanyError::NotFound);
    }
    crate::service::companies::company(store, company_id).await
}

/// Is network discovery allowed for this company?
pub async fn allowed(store: &dyn Storage, company_id: &str) -> anyhow::Result<bool> {
    Ok(store
        .company(company_id)
        .await?
        .is_some_and(|detail| detail.company.discovery_allowed))
}

/// What a check of `device` names it by: its address. A name would follow
/// the device when its address changes, but only where the probe's machine
/// resolves it; the address is what was found.
fn host(device: &Device) -> &str {
    &device.address
}

/// What people call `device`: its name without the domain, or its address.
fn label(device: &Device) -> String {
    device
        .name
        .as_deref()
        .and_then(|name| name.split('.').next())
        .filter(|short| !short.is_empty())
        .unwrap_or(&device.address)
        .to_string()
}

/// Does `spec` look at `device`? Its target names the device's address or
/// its name, as a whole: `10.0.0.5`, `10.0.0.5:443`, `https://10.0.0.5/…`,
/// not `10.0.0.50`.
fn watches(spec: &CheckSpec, device: &Device) -> bool {
    let target = spec.target().to_lowercase();
    let name = device.name.as_deref().map(str::to_lowercase);
    target
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .map(|part| part.trim_end_matches('.'))
        .any(|part| part == device.address || Some(part) == name.as_deref())
}

/// The check that suits what listens on `port`: the kind that speaks its
/// protocol where the probe has one that needs no login, else the port.
fn port_check(host: &str, port: u16) -> CheckSpec {
    let host = host.to_string();
    let database = |engine: DbEngine| CheckSpec::Database {
        engine,
        host: host.clone(),
        port: None,
    };
    match port {
        389 => CheckSpec::Ldap {
            host,
            port: None,
            security: LdapSecurity::None,
            verify: true,
        },
        1433 => database(DbEngine::Mssql),
        3306 => database(DbEngine::Mysql),
        5432 => database(DbEngine::Postgres),
        _ => CheckSpec::Tcp { host, port },
    }
}

/// The checks `device` suggests that `existing` doesn't have yet: a ping,
/// and one for each port it listens on.
pub fn suggest(device: &Device, existing: &[CheckSpec]) -> Vec<Suggestion> {
    let (host, label) = (host(device), label(device));
    let mut suggestions = Vec::new();
    // A device that answers no ping would only be Down: its ports say more.
    if device.pinged {
        suggestions.push(Suggestion {
            name: label.clone(),
            spec: CheckSpec::ping(host),
            why: "it answers a ping: is it there?".to_string(),
        });
    }
    for port in &device.ports {
        let what = port_label(*port).unwrap_or("port");
        suggestions.push(Suggestion {
            name: format!("{label} {what}"),
            spec: port_check(host, *port),
            why: format!("it listens on port {port} ({what})"),
        });
    }
    suggestions.retain(|s| s.spec.validate().is_ok() && !existing.contains(&s.spec));
    suggestions
}

/// A sweep as the probe's page shows it.
fn shown(found: Discovery, existing: &[CheckSpec]) -> Vec<FoundDevice> {
    found
        .devices
        .into_iter()
        .map(|device| FoundDevice {
            checks: existing
                .iter()
                .filter(|spec| watches(spec, &device))
                .count(),
            suggestions: suggest(&device, existing),
            device,
        })
        .collect()
}

/// What a probe last found on its network, and whether it may look.
pub async fn network(
    store: &dyn Storage,
    scope: &CompanyScope,
    probe_id: &str,
) -> ProbeResult<NetworkInfo> {
    let record = scoped_record(store, scope, probe_id).await?;
    let allowed = allowed(store, &record.company_id).await?;
    let Some((found, at)) = store.probe_discovery(probe_id).await? else {
        return Ok(NetworkInfo {
            allowed,
            ..NetworkInfo::default()
        });
    };
    let existing: Vec<CheckSpec> = store
        .probe_checks(probe_id)
        .await?
        .into_iter()
        .map(|check| check.spec)
        .collect();
    Ok(NetworkInfo {
        allowed,
        reported_at: Some(at.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        networks: found.networks.clone(),
        scanned: found.scanned,
        seconds: (found.finished_at - found.started_at).num_seconds().max(0) as u32,
        note: found.note.clone(),
        devices: shown(found, &existing),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(address: &str, name: Option<&str>, pinged: bool, ports: &[u16]) -> Device {
        Device {
            address: address.into(),
            name: name.map(str::to_string),
            mac: None,
            pinged,
            ports: ports.to_vec(),
        }
    }

    #[test]
    fn a_device_suggests_a_ping_and_a_check_of_each_port() {
        let nas = device("192.168.1.20", Some("nas.lan"), true, &[445, 5000, 5432]);
        let suggested = suggest(&nas, &[]);
        let named: Vec<(&str, &str)> = suggested
            .iter()
            .map(|s| (s.name.as_str(), s.spec.kind()))
            .collect();
        assert_eq!(
            named,
            [
                ("nas", "ping"),
                ("nas Windows file sharing", "tcp"),
                ("nas NAS web interface", "tcp"),
                ("nas PostgreSQL", "database"),
            ]
        );
        // By the address that was found, not the name.
        assert_eq!(suggested[0].spec, CheckSpec::ping("192.168.1.20"));
        assert_eq!(
            suggested[1].spec,
            CheckSpec::Tcp {
                host: "192.168.1.20".into(),
                port: 445
            }
        );
        assert!(suggested[3].why.contains("5432"));
    }

    #[test]
    fn what_the_probe_already_checks_isnt_suggested_again() {
        let printer = device("192.168.1.30", None, true, &[80, 9100]);
        let existing = [
            CheckSpec::ping("192.168.1.30"),
            CheckSpec::Tcp {
                host: "192.168.1.30".into(),
                port: 9100,
            },
        ];
        let suggested = suggest(&printer, &existing);
        assert_eq!(suggested.len(), 1);
        assert_eq!(suggested[0].name, "192.168.1.30 HTTP");
    }

    #[test]
    fn a_device_that_answers_no_ping_gets_no_ping_check() {
        let pc = device("192.168.1.40", Some("desk-7"), false, &[3389]);
        let suggested = suggest(&pc, &[]);
        assert_eq!(suggested.len(), 1);
        assert_eq!(suggested[0].name, "desk-7 Remote Desktop");

        let ldap = suggest(&device("10.0.0.2", None, false, &[389, 1433]), &[]);
        assert_eq!(ldap[0].spec.kind(), "ldap");
        assert_eq!(ldap[1].spec.kind(), "database");
        // Nothing found but a hardware address: nothing to suggest.
        assert!(suggest(&device("10.0.0.3", None, false, &[]), &[]).is_empty());
    }

    #[test]
    fn checks_that_name_a_device_are_counted() {
        let nas = device("192.168.1.2", Some("nas.lan"), true, &[]);
        let http = |url: &str| CheckSpec::Http {
            url: url.into(),
            options: Default::default(),
        };
        for spec in [
            CheckSpec::ping("192.168.1.2"),
            CheckSpec::ping("NAS.lan"),
            CheckSpec::Tcp {
                host: "192.168.1.2".into(),
                port: 445,
            },
            http("https://192.168.1.2:5001/"),
            http("http://nas.lan/status"),
        ] {
            assert!(watches(&spec, &nas), "{}", spec.target());
        }
        for spec in [
            CheckSpec::ping("192.168.1.20"),
            CheckSpec::ping("192.168.1.200"),
            CheckSpec::ping("10.192.168.1.2"),
            CheckSpec::ping("192.168.1.2.example.com"),
            CheckSpec::ping("mynas.lan"),
            CheckSpec::ping("nas.lan.example.com"),
            http("http://192.168.1.21/"),
            CheckSpec::Cpu,
        ] {
            assert!(!watches(&spec, &nas), "{}", spec.target());
        }
        let found = Discovery {
            started_at: "2026-10-14T09:00:00Z".parse().unwrap(),
            finished_at: "2026-10-14T09:00:09Z".parse().unwrap(),
            networks: vec![],
            scanned: 1,
            devices: vec![nas],
            note: None,
        };
        let listed = shown(found, &[CheckSpec::ping("192.168.1.2"), CheckSpec::Cpu]);
        assert_eq!(listed[0].checks, 1);
        assert!(listed[0].suggestions.is_empty());
    }
}
