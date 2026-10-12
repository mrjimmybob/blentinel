//! What this machine is made of, for the hub (M5-12, D-85): its system,
//! fixed disks, running services, network interfaces and default gateway.
//! The hub shows it and suggests checks from it.
//!
//! Read natively (FP-2): `sysinfo`, the Service Control Manager or systemd
//! (over D-Bus, else the files it writes), the adapters' table on Windows
//! and `/proc/net/route` on Linux. A part that can't be read is left empty:
//! the inventory helps setting a probe up, it must never stop a report.

use chrono::{DateTime, Duration, Utc};
use common::inventory::{Interface, Inventory, Os, OsFamily, Service};
use sysinfo::System;

use crate::monitor::Monitor;

/// How often the machine is looked at again.
const LOOK_EVERY_MINUTES: i64 = 60;
/// An unchanged inventory is sent again after this long: the hub may have
/// lost the last one (a restored backup, a report it refused).
const RESEND_AFTER_HOURS: i64 = 24;

/// Decides when a report carries the inventory: the first one after the
/// probe starts, then when the machine changed, and once a day.
#[derive(Debug, Default)]
pub struct Collector {
    looked_at: Option<DateTime<Utc>>,
    sent: Option<(Inventory, DateTime<Utc>)>,
}

impl Collector {
    /// The inventory for a report made at `now`, if it should carry one.
    pub async fn due(&mut self, monitor: &Monitor, now: DateTime<Utc>) -> Option<Inventory> {
        if !self.look_now(now) {
            return None;
        }
        let current = collect(monitor).await;
        self.send_if_due(current, now)
    }

    /// Is it time to look at the machine again?
    fn look_now(&mut self, now: DateTime<Utc>) -> bool {
        let due = self
            .looked_at
            .is_none_or(|at| now - at >= Duration::minutes(LOOK_EVERY_MINUTES));
        if due {
            self.looked_at = Some(now);
        }
        due
    }

    /// `current` if the hub should get it: it differs from what was sent
    /// last, or that was long ago.
    fn send_if_due(&mut self, current: Inventory, now: DateTime<Utc>) -> Option<Inventory> {
        let fresh = self.sent.as_ref().is_some_and(|(sent, at)| {
            *sent == current && now - *at < Duration::hours(RESEND_AFTER_HOURS)
        });
        if fresh {
            return None;
        }
        self.sent = Some((current.clone(), now));
        Some(current)
    }
}

/// Look at the machine.
async fn collect(monitor: &Monitor) -> Inventory {
    let volumes = crate::checks::disks::inventory(&*monitor.disk_info.read().await);
    let (interfaces, gateway) = tokio::task::spawn_blocking(platform::network)
        .await
        .unwrap_or_default();
    Inventory {
        os: os(),
        volumes,
        services: platform::services().await,
        interfaces,
        gateway,
    }
    .bounded()
}

fn os() -> Os {
    let family = if cfg!(windows) {
        OsFamily::Windows
    } else if cfg!(target_os = "linux") {
        OsFamily::Linux
    } else {
        OsFamily::Other
    };
    Os {
        family,
        name: os_name(
            family,
            System::name(),
            System::os_version(),
            System::long_os_version(),
        ),
        version: System::kernel_version().unwrap_or_default(),
        arch: System::cpu_arch(),
    }
}

/// `Ubuntu 26.04`, `Windows 11 Pro`: Windows describes itself best in its
/// long version; a Linux distribution by its name and release.
fn os_name(
    family: OsFamily,
    name: Option<String>,
    version: Option<String>,
    long: Option<String>,
) -> String {
    let short = [name, version]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    match (family, long) {
        (OsFamily::Windows, Some(long)) if !long.trim().is_empty() => long,
        (_, long) if short.is_empty() => long.unwrap_or_default(),
        _ => short,
    }
}

/// Is this an address worth showing: not the machine talking to itself?
fn shown(address: &std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local(),
        // fe80::/10: link-local.
        std::net::IpAddr::V6(v6) => !v6.is_loopback() && (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

fn interface(name: &str, addresses: impl Iterator<Item = std::net::IpAddr>) -> Option<Interface> {
    let addresses: Vec<String> = addresses
        .filter(shown)
        .map(|address| address.to_string())
        .collect();
    (!addresses.is_empty()).then(|| Interface {
        name: name.to_string(),
        addresses,
    })
}

#[cfg(target_os = "linux")]
mod platform {
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::Path;

    use super::{Interface, Service, interface};

    /// Where systemd keeps a link per running service (`checks/service.rs`).
    const ACTIVE_UNITS: &str = "/run/systemd/units";

    /// The running services: from systemd over D-Bus, else from its files.
    pub async fn services() -> Vec<Service> {
        match crate::ops::running_services().await {
            Ok(Some(services)) => services
                .into_iter()
                .map(|(name, description)| Service {
                    label: (!description.is_empty() && description != name).then_some(description),
                    name,
                })
                .collect(),
            Ok(None) => from_files(Path::new(ACTIVE_UNITS)),
            Err(e) => {
                tracing::debug!("Can't list the services: {e:#}");
                Vec::new()
            }
        }
    }

    /// The services with a link in `active_units`: those that run, by name
    /// only. For a probe that isn't let onto the system bus.
    pub(super) fn from_files(active_units: &Path) -> Vec<Service> {
        let Ok(entries) = std::fs::read_dir(active_units) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let unit = name.strip_prefix("invocation:")?;
                unit.ends_with(".service").then(|| Service {
                    name: unit.to_string(),
                    label: None,
                })
            })
            .collect()
    }

    /// The gateway of the default route in this text of `/proc/net/route`:
    /// the line whose destination is 0.0.0.0, its gateway in hexadecimal as
    /// the machine keeps it in memory.
    pub(super) fn default_gateway(routes: &str) -> Option<Ipv4Addr> {
        routes.lines().skip(1).find_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            let (destination, gateway) = (fields.next()?, fields.next()?);
            if destination != "00000000" {
                return None;
            }
            let gateway = u32::from_str_radix(gateway, 16).ok()?;
            (gateway != 0).then(|| Ipv4Addr::from(gateway.to_ne_bytes()))
        })
    }

    pub fn network() -> (Vec<Interface>, Option<String>) {
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let interfaces = networks
            .iter()
            .filter_map(|(name, data)| interface(name, data.ip_networks().iter().map(|n| n.addr)))
            .collect();
        let gateway = std::fs::read_to_string("/proc/net/route")
            .ok()
            .and_then(|routes| default_gateway(&routes))
            .map(|gateway| IpAddr::V4(gateway).to_string());
        (interfaces, gateway)
    }
}

#[cfg(windows)]
mod platform {
    use super::{Interface, Service, interface};

    pub async fn services() -> Vec<Service> {
        let listed = tokio::task::spawn_blocking(common::service::windows::running_services).await;
        match listed {
            Ok(Ok(services)) => services
                .into_iter()
                .map(|(name, label)| Service {
                    label: (!label.is_empty() && label != name).then_some(label),
                    name,
                })
                .collect(),
            Ok(Err(e)) => {
                tracing::debug!("Can't list the services: {e}");
                Vec::new()
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn network() -> (Vec<Interface>, Option<String>) {
        let Ok(adapters) = ipconfig::get_adapters() else {
            return (Vec::new(), None);
        };
        let up: Vec<_> = adapters
            .iter()
            .filter(|a| {
                a.oper_status() == ipconfig::OperStatus::IfOperStatusUp
                    && a.if_type() != ipconfig::IfType::SoftwareLoopback
            })
            .collect();
        let interfaces = up
            .iter()
            .filter_map(|a| interface(a.friendly_name(), a.ip_addresses().iter().copied()))
            .collect();
        let gateway = up
            .iter()
            .flat_map(|a| a.gateways())
            .find(|gateway| gateway.is_ipv4())
            .map(|gateway| gateway.to_string());
        (interfaces, gateway)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::{Interface, Service};

    pub async fn services() -> Vec<Service> {
        Vec::new()
    }

    pub fn network() -> (Vec<Interface>, Option<String>) {
        (Vec::new(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn machine(gateway: &str) -> Inventory {
        Inventory {
            gateway: Some(gateway.into()),
            ..Inventory::default()
        }
    }

    #[test]
    fn the_inventory_is_sent_at_the_start_when_it_changed_and_once_a_day() {
        let mut collector = Collector::default();
        let start = at("2026-10-12T08:00:00Z");
        assert!(collector.look_now(start));
        assert!(collector.send_if_due(machine("10.0.0.1"), start).is_some());

        // Not looked at again within the hour.
        assert!(!collector.look_now(start + Duration::minutes(59)));
        // The same machine an hour later: nothing to send.
        let later = start + Duration::hours(1);
        assert!(collector.look_now(later));
        assert_eq!(collector.send_if_due(machine("10.0.0.1"), later), None);
        // A change is sent.
        let changed = start + Duration::hours(2);
        assert_eq!(
            collector.send_if_due(machine("10.0.0.254"), changed),
            Some(machine("10.0.0.254"))
        );
        // And unchanged, again a day after it was last sent.
        assert_eq!(
            collector.send_if_due(machine("10.0.0.254"), changed + Duration::hours(23)),
            None
        );
        assert!(
            collector
                .send_if_due(machine("10.0.0.254"), changed + Duration::hours(24))
                .is_some()
        );
    }

    #[test]
    fn a_system_is_named_as_people_know_it() {
        let name = |family, name: &str, version: &str, long: &str| {
            let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
            os_name(family, some(name), some(version), some(long))
        };
        assert_eq!(
            name(OsFamily::Linux, "Ubuntu", "26.04", "Linux (Ubuntu 26.04)"),
            "Ubuntu 26.04"
        );
        assert_eq!(
            name(OsFamily::Windows, "Windows", "11 (26100)", "Windows 11 Pro"),
            "Windows 11 Pro"
        );
        assert_eq!(name(OsFamily::Other, "", "", "FreeBSD 15"), "FreeBSD 15");
        assert_eq!(name(OsFamily::Linux, "", "", ""), "");
    }

    #[test]
    fn an_interface_shows_the_addresses_others_reach_it_by() {
        let addresses = [
            "127.0.0.1",
            "169.254.3.4",
            "192.168.1.20",
            "fe80::1",
            "::1",
            "2001:db8::5",
        ]
        .map(|a| a.parse().unwrap());
        let shown = interface("eth0", addresses.into_iter()).unwrap();
        assert_eq!(shown.addresses, ["192.168.1.20", "2001:db8::5"]);
        // One that only talks to itself isn't listed.
        assert_eq!(
            interface("lo", ["127.0.0.1".parse().unwrap()].into_iter()),
            None
        );
    }

    /// This machine, for real: it has a system and a disk, and (on Windows
    /// and wherever systemd runs) services.
    #[tokio::test]
    async fn this_machine_has_an_inventory() {
        let inventory = collect(&Monitor::new()).await;
        assert!(!inventory.os.name.is_empty(), "{inventory:?}");
        assert!(!inventory.os.arch.is_empty());
        assert_eq!(inventory.clone().bounded(), inventory);
        if cfg!(windows) || std::path::Path::new("/run/systemd/system").exists() {
            assert!(!inventory.volumes.is_empty(), "{inventory:?}");
            assert!(!inventory.services.is_empty(), "{inventory:?}");
        }
        if cfg!(windows) {
            assert_eq!(inventory.os.family, OsFamily::Windows);
            let event_log = inventory
                .services
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case("EventLog"));
            assert!(event_log.is_some_and(|s| s.label.is_some()));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_default_gateway_is_read_from_the_routing_table() {
        let routes = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                      eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
                      eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
        // As on the machines the probe runs on (little-endian).
        if cfg!(target_endian = "little") {
            assert_eq!(
                platform::default_gateway(routes),
                Some("192.168.1.1".parse().unwrap())
            );
        }
        assert_eq!(platform::default_gateway("Iface\tDestination\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn without_the_bus_the_running_services_are_read_from_systemds_files() {
        let dir = tempfile::tempdir().unwrap();
        for link in ["invocation:nginx.service", "invocation:data.mount", "other"] {
            std::os::unix::fs::symlink("03c7d635925f4695bc3c6d6bc73f816f", dir.path().join(link))
                .unwrap();
        }
        let services = platform::from_files(dir.path());
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "nginx.service");
        assert!(platform::from_files(&dir.path().join("none")).is_empty());
    }
}
