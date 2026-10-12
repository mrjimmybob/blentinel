//! A probe's machine and the checks it suggests (M5-12, proposal 0006 §4,
//! D-85).
//!
//! The probe says what its machine is made of (`common::inventory`); the hub
//! keeps the latest and proposes checks from it: the basics every machine
//! deserves, a ping of its gateway, and the services it runs that people
//! usually want watched. Nothing is added until someone accepts it.

use common::checks::CheckSpec;
use common::inventory::{Inventory, OsFamily};

use crate::model::{CompanyScope, MachineInfo, Suggestion};
use crate::service::probes::{ProbeResult, scoped_record};
use crate::storage::Storage;

/// The most suggestions shown at once.
const MAX_SUGGESTIONS: usize = 40;

/// Windows services worth a check when they run: service name (a trailing
/// `*`: any name starting so) and what people call it.
const WINDOWS_SERVICES: [(&str, &str); 22] = [
    ("MSSQLSERVER", "SQL Server"),
    ("MSSQL$*", "SQL Server"),
    ("SQLSERVERAGENT", "SQL Server Agent"),
    ("SQLAgent$*", "SQL Server Agent"),
    ("MySQL*", "MySQL"),
    ("postgresql*", "PostgreSQL"),
    ("W3SVC", "IIS web server"),
    ("NTDS", "Active Directory"),
    ("DNS", "DNS server"),
    ("DHCPServer", "DHCP server"),
    ("Netlogon", "Netlogon"),
    ("Kdc", "Kerberos"),
    ("DFSR", "DFS replication"),
    ("CertSvc", "Certificate services"),
    ("MSExchangeIS", "Exchange"),
    ("MSExchangeTransport", "Exchange transport"),
    ("vmms", "Hyper-V"),
    ("Veeam*", "Veeam"),
    ("TermServLicensing", "Remote Desktop licensing"),
    ("WsusService", "WSUS"),
    ("wbengine", "Windows Server Backup"),
    ("TSGateway", "Remote Desktop gateway"),
];

/// The same for systemd units, without `.service`.
const LINUX_SERVICES: [(&str, &str); 26] = [
    ("nginx", "nginx"),
    ("apache2", "Apache"),
    ("httpd", "Apache"),
    ("caddy", "Caddy"),
    ("haproxy", "HAProxy"),
    ("mysql", "MySQL"),
    ("mysqld", "MySQL"),
    ("mariadb", "MariaDB"),
    ("postgresql@*", "PostgreSQL"),
    ("postgresql", "PostgreSQL"),
    ("redis-server", "Redis"),
    ("redis", "Redis"),
    ("mongod", "MongoDB"),
    ("docker", "Docker"),
    ("k3s", "k3s"),
    ("libvirtd", "libvirt"),
    ("pveproxy", "Proxmox"),
    ("smbd", "Samba"),
    ("nfs-server", "NFS server"),
    ("named", "DNS server"),
    ("bind9", "DNS server"),
    ("postfix", "Postfix"),
    ("dovecot", "Dovecot"),
    ("cups", "Printing"),
    ("veeamservice", "Veeam"),
    ("blentinel-hub", "Blentinel hub"),
];

/// Does `name` match `pattern` (a trailing `*`: starts with), whatever the case?
fn matches(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (pattern.to_lowercase(), name.to_lowercase());
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// A service's name as checks compare it: `nginx.service` is `nginx`.
fn bare(name: &str) -> String {
    name.strip_suffix(".service").unwrap_or(name).to_lowercase()
}

fn suggestion(name: impl Into<String>, spec: CheckSpec, why: &str) -> Suggestion {
    Suggestion {
        name: name.into(),
        spec,
        why: why.to_string(),
    }
}

/// The checks `inventory` suggests that `existing` doesn't have yet.
pub fn suggest(inventory: &Inventory, existing: &[CheckSpec]) -> Vec<Suggestion> {
    let has = |kind: &str| existing.iter().any(|spec| spec.kind() == kind);
    let mut suggestions = Vec::new();

    if !has("cpu") {
        suggestions.push(suggestion("CPU", CheckSpec::Cpu, "every machine"));
    }
    if !has("memory") {
        suggestions.push(suggestion("Memory", CheckSpec::Memory, "every machine"));
    }
    if !has("disks") && !has("disk") && !inventory.volumes.is_empty() {
        let mounts: Vec<&str> = inventory.volumes.iter().map(|v| v.mount.as_str()).collect();
        suggestions.push(suggestion(
            "Disks",
            CheckSpec::Disks { ignore: vec![] },
            &format!("its fixed disks: {}", mounts.join(", ")),
        ));
    }
    if !has("uptime") {
        suggestions.push(suggestion("Uptime", CheckSpec::Uptime, "every machine"));
    }

    if let Some(gateway) = &inventory.gateway {
        let pinged = existing
            .iter()
            .any(|spec| matches!(spec, CheckSpec::Ping { host, .. } if host == gateway));
        if !pinged {
            suggestions.push(suggestion(
                "Gateway",
                CheckSpec::ping(gateway),
                "the machine's default gateway: is its network up?",
            ));
        }
    }

    let known: &[(&str, &str)] = match inventory.os.family {
        OsFamily::Windows => &WINDOWS_SERVICES,
        OsFamily::Linux => &LINUX_SERVICES,
        OsFamily::Other => &[],
    };
    let checked: Vec<String> = existing
        .iter()
        .filter_map(|spec| match spec {
            CheckSpec::Service { name } => Some(bare(name)),
            _ => None,
        })
        .collect();
    for service in &inventory.services {
        let name = bare(&service.name);
        let Some((_, label)) = known.iter().find(|(pattern, _)| matches(pattern, &name)) else {
            continue;
        };
        if checked.contains(&name) {
            continue;
        }
        // The name as the machine has it, less `.service`: what the check takes.
        let unit = service
            .name
            .strip_suffix(".service")
            .unwrap_or(&service.name);
        let title = if label.eq_ignore_ascii_case(unit) {
            (*label).to_string()
        } else {
            format!("{label} ({unit})")
        };
        suggestions.push(suggestion(
            title,
            CheckSpec::Service {
                name: unit.to_string(),
            },
            "a service running on this machine",
        ));
    }

    suggestions.retain(|s| s.spec.validate().is_ok());
    suggestions.truncate(MAX_SUGGESTIONS);
    suggestions
}

/// What a probe's machine is made of, and the checks that suggests.
pub async fn machine(
    store: &dyn Storage,
    scope: &CompanyScope,
    probe_id: &str,
) -> ProbeResult<MachineInfo> {
    scoped_record(store, scope, probe_id).await?;
    let Some((inventory, at)) = store.probe_inventory(probe_id).await? else {
        return Ok(MachineInfo::default());
    };
    let existing: Vec<CheckSpec> = store
        .probe_checks(probe_id)
        .await?
        .into_iter()
        .map(|check| check.spec)
        .collect();
    Ok(MachineInfo {
        suggestions: suggest(&inventory, &existing),
        reported_at: Some(at.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        inventory: Some(inventory),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::inventory::{Os, Service, Volume};

    fn machine(family: OsFamily, services: &[&str], gateway: Option<&str>) -> Inventory {
        Inventory {
            os: Os {
                family,
                name: "x".into(),
                ..Os::default()
            },
            volumes: vec![Volume {
                mount: "/".into(),
                file_system: "ext4".into(),
                total_mb: 1000,
            }],
            services: services
                .iter()
                .map(|name| Service {
                    name: (*name).into(),
                    label: None,
                })
                .collect(),
            interfaces: vec![],
            gateway: gateway.map(str::to_string),
        }
    }

    fn names(suggestions: &[Suggestion]) -> Vec<&str> {
        suggestions.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn a_new_machine_gets_the_basics_its_gateway_and_its_known_services() {
        let inventory = machine(
            OsFamily::Linux,
            &[
                "nginx.service",
                "postgresql@16-main.service",
                "systemd-journald.service",
                "user@1000.service",
            ],
            Some("192.168.1.1"),
        );
        let suggestions = suggest(&inventory, &[]);
        assert_eq!(
            names(&suggestions),
            [
                "CPU",
                "Memory",
                "Disks",
                "Uptime",
                "Gateway",
                "nginx",
                "PostgreSQL (postgresql@16-main)"
            ]
        );
        assert_eq!(suggestions[4].spec, CheckSpec::ping("192.168.1.1"));
        assert_eq!(
            suggestions[6].spec,
            CheckSpec::Service {
                name: "postgresql@16-main".into()
            }
        );
        assert!(suggestions[2].why.contains('/'), "{}", suggestions[2].why);
    }

    #[test]
    fn what_the_probe_already_checks_isnt_suggested_again() {
        let inventory = machine(OsFamily::Linux, &["nginx.service"], Some("192.168.1.1"));
        let existing = [
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk { path: None },
            CheckSpec::Uptime,
            CheckSpec::ping("192.168.1.1"),
            // As someone typed it, with the suffix and another case.
            CheckSpec::Service {
                name: "Nginx.service".into(),
            },
        ];
        assert!(suggest(&inventory, &existing).is_empty());
        // Another ping doesn't stand for the gateway's.
        let other = [existing[0].clone(), CheckSpec::ping("10.0.0.9")];
        assert!(names(&suggest(&inventory, &other)).contains(&"Gateway"));
    }

    #[test]
    fn windows_services_are_matched_by_service_name() {
        let inventory = machine(
            OsFamily::Windows,
            &["MSSQL$SQLEXPRESS", "w3svc", "Spooler", "VeeamBackupSvc"],
            None,
        );
        let suggestions = suggest(&inventory, &[CheckSpec::Cpu]);
        assert_eq!(
            names(&suggestions),
            [
                "Memory",
                "Disks",
                "Uptime",
                "SQL Server (MSSQL$SQLEXPRESS)",
                "IIS web server (w3svc)",
                "Veeam (VeeamBackupSvc)"
            ]
        );
        // A Linux name means nothing on Windows, and the other way round.
        let linux = machine(OsFamily::Linux, &["W3SVC"], None);
        assert_eq!(suggest(&linux, &[CheckSpec::Cpu]).len(), 3);
        // A system this hub doesn't know: the basics only.
        let other = machine(OsFamily::Other, &["nginx"], None);
        assert_eq!(
            names(&suggest(&other, &[])),
            ["CPU", "Memory", "Disks", "Uptime"]
        );
    }

    #[test]
    fn every_suggestion_is_a_check_the_hub_accepts() {
        let services: Vec<String> = LINUX_SERVICES
            .iter()
            .map(|(pattern, _)| format!("{}.service", pattern.replace('*', "x")))
            .collect();
        let services: Vec<&str> = services.iter().map(String::as_str).collect();
        let inventory = machine(OsFamily::Linux, &services, Some("10.0.0.1"));
        let suggestions = suggest(&inventory, &[]);
        assert_eq!(suggestions.len(), 5 + LINUX_SERVICES.len());
        for suggestion in &suggestions {
            assert_eq!(suggestion.spec.validate(), Ok(()), "{}", suggestion.name);
            assert_eq!(
                common::checks::validate_name(&suggestion.name),
                Ok(()),
                "{}",
                suggestion.name
            );
        }
    }
}
