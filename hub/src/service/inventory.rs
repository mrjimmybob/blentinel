//! A probe's machine and the checks it suggests (M5-12, proposal 0006 §4,
//! D-85).
//!
//! The probe says what its machine is made of (`common::inventory`); the hub
//! keeps the latest and proposes checks from it: the basics every machine
//! deserves, a ping of its gateway, and the services it runs that people
//! usually want watched. Nothing is added until someone accepts it.

use common::checks::{CheckSpec, DbEngine};
use common::inventory::{Inventory, OsFamily};

use crate::model::{CompanyScope, MachineInfo, Suggestion};
use crate::service::probes::{ProbeResult, scoped_record};
use crate::storage::Storage;

/// The most suggestions shown at once.
const MAX_SUGGESTIONS: usize = 40;

/// Windows services worth a check when they run: service name (a trailing
/// `*`: any name starting so) and what people call it.
const WINDOWS_SERVICES: [(&str, &str); 23] = [
    ("MSSQLSERVER", "SQL Server"),
    ("MSSQL$*", "SQL Server"),
    ("SQLSERVERAGENT", "SQL Server Agent"),
    ("SQLAgent$*", "SQL Server Agent"),
    ("MySQL*", "MySQL"),
    ("postgresql*", "PostgreSQL"),
    ("MongoDB", "MongoDB"),
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

/// The database a known service is, where a `database` check on the
/// machine itself is likely right: the engine's usual port. Not a named SQL
/// Server instance (`MSSQL$…`), which listens elsewhere.
fn engine_of(label: &str, service: &str) -> Option<DbEngine> {
    match label {
        "PostgreSQL" => Some(DbEngine::Postgres),
        "MySQL" | "MariaDB" => Some(DbEngine::Mysql),
        "MongoDB" => Some(DbEngine::Mongodb),
        "SQL Server" if service.eq_ignore_ascii_case("MSSQLSERVER") => Some(DbEngine::Mssql),
        _ => None,
    }
}

/// The address a check of the probe's own machine uses.
const THIS_MACHINE: &str = "127.0.0.1";

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
    if !has("updates") && inventory.os.family != OsFamily::Other {
        suggestions.push(suggestion(
            "Pending updates",
            CheckSpec::updates(),
            "security updates that wait, and a restart that is needed (only reported)",
        ));
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
    // The databases among the services: one check of each engine.
    let mut engines: Vec<DbEngine> = Vec::new();
    for service in &inventory.services {
        let name = bare(&service.name);
        let Some((_, label)) = known.iter().find(|(pattern, _)| matches(pattern, &name)) else {
            continue;
        };
        if let Some(engine) = engine_of(label, &name)
            && !engines.contains(&engine)
        {
            engines.push(engine);
        }
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

    for engine in engines {
        // Whatever the host and port of a check of that engine: it is watched.
        let watched = existing
            .iter()
            .any(|spec| matches!(spec, CheckSpec::Database { engine: has, .. } if *has == engine));
        if !watched {
            suggestions.push(suggestion(
                format!("{} connections", engine.label()),
                CheckSpec::Database {
                    engine,
                    host: THIS_MACHINE.to_string(),
                    port: None,
                },
                &format!(
                    "a database running on this machine: does it accept connections (port {})?",
                    engine.default_port()
                ),
            ));
        }
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
                "Pending updates",
                "Gateway",
                "nginx",
                "PostgreSQL (postgresql@16-main)",
                "PostgreSQL connections"
            ]
        );
        assert_eq!(suggestions[4].spec, CheckSpec::updates());
        assert_eq!(
            suggestions[8].spec,
            CheckSpec::Database {
                engine: DbEngine::Postgres,
                host: "127.0.0.1".into(),
                port: None,
            }
        );
        assert!(suggestions[8].why.contains("port 5432"));
        assert_eq!(suggestions[5].spec, CheckSpec::ping("192.168.1.1"));
        assert_eq!(
            suggestions[7].spec,
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
            // With limits of its own: still the one updates check.
            CheckSpec::Updates {
                security_days: Some(7),
                reboot_days: None,
                stale_days: None,
            },
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
    fn a_database_among_the_services_gets_a_check_of_its_connections_once() {
        let inventory = machine(
            OsFamily::Linux,
            &[
                "postgresql.service",
                "postgresql@16-main.service",
                "mariadb.service",
                "mongod.service",
                "redis-server.service",
            ],
            None,
        );
        let databases = |existing: &[CheckSpec]| -> Vec<String> {
            suggest(&inventory, existing)
                .into_iter()
                .filter(|s| s.spec.kind() == "database")
                .map(|s| s.name)
                .collect()
        };
        assert_eq!(
            databases(&[]),
            [
                "PostgreSQL connections",
                "MySQL or MariaDB connections",
                "MongoDB connections"
            ]
        );
        // One that is watched already, wherever, isn't proposed again; nor
        // does watching its service stand for it.
        let existing = [
            CheckSpec::Database {
                engine: DbEngine::Postgres,
                host: "db.lan".into(),
                port: Some(5433),
            },
            CheckSpec::Service {
                name: "mariadb".into(),
            },
        ];
        assert_eq!(
            databases(&existing),
            ["MySQL or MariaDB connections", "MongoDB connections"]
        );

        // Windows: the default SQL Server instance, not a named one.
        let default = machine(OsFamily::Windows, &["MSSQLSERVER", "MongoDB"], None);
        let named = machine(OsFamily::Windows, &["MSSQL$SQLEXPRESS"], None);
        let of = |inventory: &Inventory| -> Vec<String> {
            suggest(inventory, &[])
                .into_iter()
                .filter(|s| s.spec.kind() == "database")
                .map(|s| s.name)
                .collect()
        };
        assert_eq!(
            of(&default),
            ["Microsoft SQL Server connections", "MongoDB connections"]
        );
        assert!(of(&named).is_empty());
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
                "Pending updates",
                "SQL Server (MSSQL$SQLEXPRESS)",
                "IIS web server (w3svc)",
                "Veeam (VeeamBackupSvc)"
            ]
        );
        // A Linux name means nothing on Windows, and the other way round.
        let linux = machine(OsFamily::Linux, &["W3SVC"], None);
        assert_eq!(suggest(&linux, &[CheckSpec::Cpu]).len(), 4);
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
        // The basics (with the updates and the gateway), each service, and
        // the three databases among them.
        assert_eq!(suggestions.len(), 6 + LINUX_SERVICES.len() + 3);
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
