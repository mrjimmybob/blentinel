//! What a probe found on its own network (M5-18, proposal 0006 §4, D-101):
//! the devices that answered, by address, name and hardware address, and
//! which well-known ports they listen on.
//!
//! The probe looks only when its hub asks (the `discover` command), and the
//! hub asks only for a company that allows it. What it found comes with a
//! report (protocol v10); the hub keeps the latest within
//! [`Discovery::bounded`] and proposes checks from it.

use std::net::Ipv4Addr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The most addresses one sweep looks at, over all the probe's networks.
pub const MAX_ADDRESSES: u32 = 1024;

/// The most devices the hub keeps of one sweep.
pub const MAX_DEVICES: usize = MAX_ADDRESSES as usize;

/// The longest text kept: a device's name, a network, the note.
pub const MAX_TEXT_CHARS: usize = 200;

/// The most networks one sweep names.
pub const MAX_NETWORKS: usize = 16;

/// The TCP ports a sweep tries on a device that answered, and what usually
/// listens there. A short list of what people monitor: not a port scan.
pub const PORTS: [(u16, &str); 22] = [
    (21, "FTP"),
    (22, "SSH"),
    (23, "Telnet"),
    (25, "SMTP"),
    (53, "DNS"),
    (80, "HTTP"),
    (139, "NetBIOS"),
    (143, "IMAP"),
    (389, "LDAP"),
    (443, "HTTPS"),
    (445, "Windows file sharing"),
    (515, "LPD printing"),
    (631, "IPP printing"),
    (1433, "SQL Server"),
    (3306, "MySQL"),
    (3389, "Remote Desktop"),
    (5000, "NAS web interface"),
    (5432, "PostgreSQL"),
    (5900, "VNC"),
    (8080, "HTTP (8080)"),
    (8443, "HTTPS (8443)"),
    (9100, "Printer (raw)"),
];

/// What usually listens on `port`, if it is one a sweep tries.
pub fn port_label(port: u16) -> Option<&'static str> {
    PORTS
        .iter()
        .find(|(known, _)| *known == port)
        .map(|(_, label)| *label)
}

/// A device that answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// Its IPv4 address.
    pub address: String,
    /// The name its address has in the network's DNS, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its hardware address, as the probe's machine learned it:
    /// `aa:bb:cc:dd:ee:ff`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    /// It answered a ping. `false`: it was found by its hardware address
    /// or a port only (many firewalls drop pings).
    pub pinged: bool,
    /// Which of [`PORTS`] accepted a connection, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
}

/// One sweep of the probe's networks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// The networks looked at: `192.168.1.0/24`.
    #[serde(default)]
    pub networks: Vec<String>,
    /// How many addresses were tried.
    pub scanned: u32,
    #[serde(default)]
    pub devices: Vec<Device>,
    /// What people should know about this sweep: nothing to look at, a
    /// network looked at only in part, no permission to ping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn cut(text: &mut String) {
    if let Some((at, _)) = text.char_indices().nth(MAX_TEXT_CHARS) {
        text.truncate(at);
    }
}

impl Discovery {
    /// The same sweep within the limits above, its devices in the order of
    /// their addresses: what the hub keeps of whatever it was sent. A
    /// device whose address isn't an IPv4 address is dropped, and ports a
    /// sweep doesn't try.
    pub fn bounded(mut self) -> Self {
        self.networks.truncate(MAX_NETWORKS);
        self.networks.iter_mut().for_each(cut);
        if let Some(note) = &mut self.note {
            cut(note);
        }
        let mut devices: Vec<(Ipv4Addr, Device)> = self
            .devices
            .into_iter()
            .filter_map(|device| Some((device.address.parse().ok()?, device)))
            .collect();
        devices.sort_by_key(|(address, _)| *address);
        devices.dedup_by_key(|(address, _)| *address);
        devices.truncate(MAX_DEVICES);
        self.devices = devices
            .into_iter()
            .map(|(address, mut device)| {
                device.address = address.to_string();
                if let Some(name) = &mut device.name {
                    cut(name);
                }
                device.name = device.name.filter(|name| !name.trim().is_empty());
                device.mac = device.mac.as_deref().and_then(mac);
                device.ports.retain(|port| port_label(*port).is_some());
                device.ports.sort_unstable();
                device.ports.dedup();
                device
            })
            .collect();
        self
    }
}

/// A hardware address as the hub shows it: six bytes in lower-case hex with
/// colons, from `AA-BB-CC-DD-EE-FF` or `aa:bb:cc:dd:ee:ff`. `None`: not one,
/// or the address of nothing (all zeros, broadcast).
pub fn mac(text: &str) -> Option<String> {
    let bytes: Vec<u8> = text
        .split([':', '-'])
        .map(|byte| (byte.len() == 2).then(|| u8::from_str_radix(byte, 16).ok())?)
        .collect::<Option<_>>()?;
    let real = bytes.len() == 6 && bytes != [0; 6] && bytes != [0xff; 6];
    real.then(|| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    })
}

/// Is `address` one of a private network (10.x, 172.16 to 172.31.x,
/// 192.168.x)? A sweep looks nowhere else: a machine with a public address
/// has neighbours that aren't its owner's.
pub fn is_private(address: Ipv4Addr) -> bool {
    address.is_private()
}

/// The network `address` is in, by its `prefix` length, and the addresses a
/// sweep tries there: every host address but `address` itself. A network of
/// more than `room` hosts is looked at only around `address` (its /24).
/// `None`: nothing to look at (a prefix of 31 or 32, or no room).
pub fn range(address: Ipv4Addr, prefix: u8, room: u32) -> Option<Range> {
    if prefix >= 31 || room == 0 {
        return None;
    }
    let hosts = |prefix: u8| (1u32 << (32 - u32::from(prefix))) - 2;
    let whole = prefix.max(8);
    let (prefix, partial) = if whole < 24 && hosts(whole) > room {
        (24, true)
    } else {
        (whole, whole != prefix)
    };
    let mask = u32::MAX << (32 - u32::from(prefix));
    let network = u32::from(address) & mask;
    let own = u32::from(address);
    let addresses: Vec<Ipv4Addr> = (network + 1..network | !mask)
        .filter(|candidate| *candidate != own)
        .take(room as usize)
        .map(Ipv4Addr::from)
        .collect();
    Some(Range {
        network: format!("{}/{prefix}", Ipv4Addr::from(network)),
        partial: partial || (addresses.len() as u32) < hosts(prefix).saturating_sub(1),
        addresses,
    })
}

/// What [`range`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    /// `192.168.1.0/24`.
    pub network: String,
    /// Not the whole of the machine's network: it is larger than a sweep
    /// looks at.
    pub partial: bool,
    pub addresses: Vec<Ipv4Addr>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(address: &str) -> Device {
        Device {
            address: address.into(),
            name: None,
            mac: None,
            pinged: true,
            ports: vec![],
        }
    }

    fn sweep(devices: Vec<Device>) -> Discovery {
        let at: DateTime<Utc> = "2026-10-14T09:00:00Z".parse().unwrap();
        Discovery {
            started_at: at,
            finished_at: at,
            networks: vec!["192.168.1.0/24".into()],
            scanned: 253,
            devices,
            note: None,
        }
    }

    #[test]
    fn the_wire_format_leaves_out_what_a_device_doesnt_have() {
        let found = sweep(vec![
            device("192.168.1.1"),
            Device {
                name: Some("nas.lan".into()),
                mac: Some("00:11:32:aa:bb:cc".into()),
                pinged: false,
                ports: vec![445, 5000],
                ..device("192.168.1.20")
            },
        ]);
        let json = serde_json::to_string(&found).unwrap();
        assert_eq!(
            json,
            r#"{"started_at":"2026-10-14T09:00:00Z","finished_at":"2026-10-14T09:00:00Z","networks":["192.168.1.0/24"],"scanned":253,"devices":[{"address":"192.168.1.1","pinged":true},{"address":"192.168.1.20","name":"nas.lan","mac":"00:11:32:aa:bb:cc","pinged":false,"ports":[445,5000]}]}"#
        );
        assert_eq!(serde_json::from_str::<Discovery>(&json).unwrap(), found);
    }

    #[test]
    fn the_hub_keeps_a_sweep_within_bounds_and_in_order() {
        let mut many: Vec<Device> = (0..2000u32)
            .map(|n| device(&Ipv4Addr::from(0x0a00_0000 + 2000 - n).to_string()))
            .collect();
        many.push(device("not an address"));
        many.push(device("fe80::1"));
        many.push(Device {
            name: Some("n".repeat(500)),
            mac: Some("AA-BB-CC-00-11-22".into()),
            ports: vec![9100, 80, 80, 31337],
            ..device("10.0.0.1")
        });
        let mut found = sweep(many);
        found.note = Some("x".repeat(500));
        found.networks = vec!["10.0.0.0/8".repeat(100); 40];
        let kept = found.bounded();
        assert_eq!(kept.devices.len(), MAX_DEVICES);
        assert_eq!(kept.devices[0].address, "10.0.0.1");
        assert_eq!(kept.devices[1].address, "10.0.0.2");
        // The first of two with one address stays: here the plain one.
        assert_eq!(kept.devices[0].name, None);
        assert_eq!(kept.note.unwrap().len(), MAX_TEXT_CHARS);
        assert_eq!(kept.networks.len(), MAX_NETWORKS);
        assert_eq!(kept.networks[0].len(), MAX_TEXT_CHARS);

        let one = sweep(vec![Device {
            name: Some("n".repeat(500)),
            mac: Some("AA-BB-CC-00-11-22".into()),
            ports: vec![9100, 80, 80, 31337],
            ..device("10.0.0.1")
        }])
        .bounded();
        assert_eq!(one.devices[0].name.as_ref().unwrap().len(), MAX_TEXT_CHARS);
        assert_eq!(one.devices[0].mac.as_deref(), Some("aa:bb:cc:00:11:22"));
        assert_eq!(one.devices[0].ports, [80, 9100]);
    }

    #[test]
    fn a_hardware_address_is_six_real_bytes() {
        assert_eq!(
            mac("AA-BB-CC-DD-EE-0F").as_deref(),
            Some("aa:bb:cc:dd:ee:0f")
        );
        assert_eq!(
            mac("00:11:32:aa:bb:cc").as_deref(),
            Some("00:11:32:aa:bb:cc")
        );
        for not in [
            "",
            "00:00:00:00:00:00",
            "ff:ff:ff:ff:ff:ff",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:ff:00",
            "aa:bb:cc:dd:ee:zz",
            "a:b:c:d:e:f",
            "<script>",
        ] {
            assert_eq!(mac(not), None, "{not}");
        }
    }

    #[test]
    fn a_sweep_looks_at_the_machines_own_network_without_itself() {
        let home = range(Ipv4Addr::new(192, 168, 1, 10), 24, MAX_ADDRESSES).unwrap();
        assert_eq!(home.network, "192.168.1.0/24");
        assert!(!home.partial);
        assert_eq!(home.addresses.len(), 253);
        assert_eq!(home.addresses[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(home.addresses[252], Ipv4Addr::new(192, 168, 1, 254));
        assert!(!home.addresses.contains(&Ipv4Addr::new(192, 168, 1, 10)));

        let small = range(Ipv4Addr::new(10, 0, 0, 5), 29, MAX_ADDRESSES).unwrap();
        assert_eq!(small.network, "10.0.0.0/29");
        assert_eq!(small.addresses.len(), 5);

        let office = range(Ipv4Addr::new(10, 1, 2, 3), 22, MAX_ADDRESSES).unwrap();
        assert_eq!(office.network, "10.1.0.0/22");
        assert!(!office.partial);
        assert_eq!(office.addresses.len(), 1021);
    }

    #[test]
    fn a_large_network_is_looked_at_around_the_machine_only() {
        let big = range(Ipv4Addr::new(10, 20, 30, 40), 8, MAX_ADDRESSES).unwrap();
        assert_eq!(big.network, "10.20.30.0/24");
        assert!(big.partial);
        assert_eq!(big.addresses.len(), 253);

        // What is left of the sweep's room after another network.
        let rest = range(Ipv4Addr::new(192, 168, 1, 10), 24, 100).unwrap();
        assert_eq!(rest.addresses.len(), 100);
        assert!(rest.partial);

        for (prefix, room) in [(32, 1024), (31, 1024), (24, 0)] {
            assert_eq!(range(Ipv4Addr::new(10, 0, 0, 1), prefix, room), None);
        }
    }

    #[test]
    fn only_private_networks_are_looked_at() {
        for private in ["10.0.0.1", "172.16.5.1", "172.31.255.254", "192.168.0.1"] {
            assert!(is_private(private.parse().unwrap()), "{private}");
        }
        for other in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "169.254.1.1",
            "127.0.0.1",
        ] {
            assert!(!is_private(other.parse().unwrap()), "{other}");
        }
    }
}
