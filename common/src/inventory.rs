//! What a probe's machine is made of (M5-12, D-85): its operating system,
//! disks, running services, network interfaces and default gateway.
//!
//! The probe sends it with a report now and then (protocol v9); the hub
//! shows it and suggests checks from it. It is what the machine says about
//! itself: the hub keeps it within [`Inventory::bounded`] and uses it only
//! for that probe (FP-3).

use serde::{Deserialize, Serialize};

/// The most the hub keeps of each list, and of any one text.
pub const MAX_VOLUMES: usize = 64;
pub const MAX_SERVICES: usize = 1000;
pub const MAX_INTERFACES: usize = 64;
pub const MAX_ADDRESSES: usize = 16;
pub const MAX_TEXT_CHARS: usize = 200;

/// Which kind of system: it decides what a service's name means, and which
/// checks suit the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Windows,
    Linux,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Os {
    pub family: OsFamily,
    /// `Ubuntu 26.04`, `Windows 11 Pro`.
    pub name: String,
    /// The kernel or build: `6.14.0-29-generic`, `26100`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// `x86_64`, `aarch64`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arch: String,
}

/// A fixed disk, as the `disks` check counts them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// `/`, `/home`, `C:\`.
    pub mount: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_system: String,
    pub total_mb: u64,
}

/// A service that was running: `name` is what a `service` check takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// The Windows service name (`Spooler`) or the systemd unit
    /// (`nginx.service`).
    pub name: String,
    /// What Windows shows for it (`Print Spooler`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    pub name: String,
    /// Its IP addresses, without loopback and link-local ones.
    #[serde(default)]
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub os: Os,
    #[serde(default)]
    pub volumes: Vec<Volume>,
    /// The services running when it was taken.
    #[serde(default)]
    pub services: Vec<Service>,
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    /// The default gateway (IPv4), if the machine has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
}

fn cut(text: &mut String) {
    if let Some((at, _)) = text.char_indices().nth(MAX_TEXT_CHARS) {
        text.truncate(at);
    }
}

impl Inventory {
    /// The same inventory within the limits above, in a stable order: what
    /// the probe sends (so an unchanged machine sends the same again) and
    /// what the hub keeps of whatever it was sent.
    pub fn bounded(mut self) -> Self {
        cut(&mut self.os.name);
        cut(&mut self.os.version);
        cut(&mut self.os.arch);

        self.volumes.sort_by(|a, b| a.mount.cmp(&b.mount));
        self.volumes.dedup_by(|a, b| a.mount == b.mount);
        self.volumes.truncate(MAX_VOLUMES);
        for volume in &mut self.volumes {
            cut(&mut volume.mount);
            cut(&mut volume.file_system);
        }

        self.services.retain(|s| !s.name.trim().is_empty());
        self.services.sort_by_key(|s| s.name.to_lowercase());
        self.services.dedup_by(|a, b| a.name == b.name);
        self.services.truncate(MAX_SERVICES);
        for service in &mut self.services {
            cut(&mut service.name);
            if let Some(label) = &mut service.label {
                cut(label);
            }
        }

        self.interfaces.sort_by(|a, b| a.name.cmp(&b.name));
        self.interfaces.truncate(MAX_INTERFACES);
        for interface in &mut self.interfaces {
            cut(&mut interface.name);
            interface.addresses.sort();
            interface.addresses.dedup();
            interface.addresses.truncate(MAX_ADDRESSES);
            interface.addresses.iter_mut().for_each(cut);
        }

        // Only an address is a gateway: it becomes a ping's target.
        self.gateway = self
            .gateway
            .filter(|g| g.parse::<std::net::IpAddr>().is_ok());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str) -> Service {
        Service {
            name: name.into(),
            label: None,
        }
    }

    #[test]
    fn an_inventory_is_kept_within_its_limits_and_in_order() {
        let inventory = Inventory {
            os: Os {
                family: OsFamily::Linux,
                name: "x".repeat(MAX_TEXT_CHARS + 50),
                ..Os::default()
            },
            volumes: vec![],
            services: (0..MAX_SERVICES + 10)
                .map(|i| service(&format!("svc{i:05}")))
                .chain([
                    service("nginx.service"),
                    service("nginx.service"),
                    service(" "),
                ])
                .collect(),
            interfaces: vec![
                Interface {
                    name: "eth1".into(),
                    addresses: vec!["10.0.0.2".into(), "10.0.0.2".into()],
                },
                Interface {
                    name: "eth0".into(),
                    addresses: (0..40).map(|i| format!("10.0.1.{i:03}")).collect(),
                },
            ],
            gateway: Some("not an address".into()),
        }
        .bounded();

        assert_eq!(inventory.os.name.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(inventory.services.len(), MAX_SERVICES);
        assert_eq!(
            inventory
                .services
                .iter()
                .filter(|s| s.name == "nginx.service")
                .count(),
            1
        );
        assert_eq!(inventory.interfaces[0].name, "eth0");
        assert_eq!(inventory.interfaces[0].addresses.len(), MAX_ADDRESSES);
        assert_eq!(inventory.interfaces[1].addresses, ["10.0.0.2"]);
        assert_eq!(inventory.gateway, None);
        // Bounding again changes nothing: an unchanged machine sends the same.
        assert_eq!(inventory.clone().bounded(), inventory);
    }

    #[test]
    fn the_wire_format_is_pinned() {
        let inventory = Inventory {
            os: Os {
                family: OsFamily::Windows,
                name: "Windows 11 Pro".into(),
                version: "26100".into(),
                arch: "x86_64".into(),
            },
            volumes: vec![Volume {
                mount: "C:\\".into(),
                file_system: "ntfs".into(),
                total_mb: 476_000,
            }],
            services: vec![Service {
                name: "Spooler".into(),
                label: Some("Print Spooler".into()),
            }],
            interfaces: vec![Interface {
                name: "Ethernet".into(),
                addresses: vec!["192.168.1.20".into()],
            }],
            gateway: Some("192.168.1.1".into()),
        };
        let json = serde_json::to_string(&inventory).unwrap();
        assert_eq!(
            json,
            r#"{"os":{"family":"windows","name":"Windows 11 Pro","version":"26100","arch":"x86_64"},"volumes":[{"mount":"C:\\","file_system":"ntfs","total_mb":476000}],"services":[{"name":"Spooler","label":"Print Spooler"}],"interfaces":[{"name":"Ethernet","addresses":["192.168.1.20"]}],"gateway":"192.168.1.1"}"#
        );
        assert_eq!(serde_json::from_str::<Inventory>(&json).unwrap(), inventory);
        // A family this hub doesn't know is another system, not an error.
        let other: Os =
            serde_json::from_str(r#"{"family":"freebsd","name":"FreeBSD 15"}"#).unwrap();
        assert_eq!(other.family, OsFamily::Other);
    }
}
