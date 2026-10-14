//! Network discovery (M5-18, proposal 0006 §4, D-101): one look at the
//! probe's own network, when the hub asks for it (the `discover` command).
//!
//! The probe pings every address of the private networks its machine is on,
//! reads which hardware addresses the machine learned doing so, tries a
//! short list of well-known TCP ports on the devices that are there, and
//! asks the network's DNS for their names. Everything goes out from the
//! probe (FP-1) and nothing is installed or run (FP-2): the echoes and
//! connections are the probe's own, the hardware addresses are read from
//! the system's neighbour table.
//!
//! It looks only at private networks (a machine with a public address has
//! neighbours that aren't its owner's), at no more than
//! [`MAX_ADDRESSES`] addresses, and never beyond the machine's own
//! networks.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use common::discovery::{Device, Discovery, MAX_ADDRESSES, PORTS, Range, is_private, range};
use hickory_resolver::proto::rr::RecordType;
use surge_ping::{PingIdentifier, PingSequence};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::monitor::Monitor;

/// How many addresses are tried at once: a sweep is a few seconds of light
/// traffic, not a burst.
const AT_ONCE: usize = 48;
/// How long one echo may take, and how many are sent to an address.
const ECHO_TIMEOUT: Duration = Duration::from_millis(800);
const ECHOES: u16 = 2;
/// How long a TCP connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
/// How long the network's DNS may take to name an address.
const NAME_TIMEOUT: Duration = Duration::from_secs(2);
/// The port tried instead of an echo where the probe may not send any: any
/// answer, even a refusal, says the device is there.
const KNOCK_PORT: u16 = 80;
const PAYLOAD: [u8; 8] = [0; 8];

/// An interface's own address and the length of its network's prefix.
type Own = (String, Ipv4Addr, u8);

/// Interfaces that lead to no network of devices: container and virtual
/// machine bridges of the probe's own machine.
const VIRTUAL: [&str; 7] = [
    "docker",
    "br-",
    "veth",
    "virbr",
    "vethernet",
    "cni",
    "flannel",
];

/// Look at the machine's networks once.
pub async fn sweep(monitor: Arc<Monitor>) -> Discovery {
    let started_at = Utc::now();
    let (ranges, mut notes) = plan(&own_networks());
    let targets: Vec<Ipv4Addr> = ranges
        .iter()
        .flat_map(|r| r.addresses.iter().copied())
        .collect();

    let can_ping = monitor
        .icmp_client(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .is_some();
    if !can_ping && !targets.is_empty() {
        notes.push(
            "This probe may not send pings, so devices were looked for by their hardware address and port 80 only"
                .to_string(),
        );
    }
    // 1. Who answers? An echo, or a knock where the probe may not ping.
    let answered: BTreeSet<Ipv4Addr> = each(&targets, {
        let monitor = monitor.clone();
        move |address| {
            let monitor = monitor.clone();
            async move {
                if can_ping {
                    ping(&monitor, address).await
                } else {
                    knock(address, KNOCK_PORT).await != Knock::Silent
                }
            }
        }
    })
    .await
    .into_iter()
    .filter_map(|(address, there)| there.then_some(address))
    .collect();

    // 2. Whom did the machine learn the hardware address of, trying?
    let wanted: BTreeSet<Ipv4Addr> = targets.iter().copied().collect();
    let mut macs: BTreeMap<Ipv4Addr, String> = platform::neighbours()
        .await
        .into_iter()
        .filter(|(address, _)| wanted.contains(address))
        .collect();

    // 3. What do those that are there listen on, and what are they called?
    let there: Vec<Ipv4Addr> = answered
        .iter()
        .chain(macs.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut ports: HashMap<Ipv4Addr, Vec<u16>> =
        each(&there, open_ports).await.into_iter().collect();
    let resolver = crate::checks::dns::resolver(None).ok().map(Arc::new);
    let mut names: HashMap<Ipv4Addr, Option<String>> = match resolver {
        Some(resolver) => each(&there, move |address| {
            let resolver = resolver.clone();
            async move { name_of(&resolver, address).await }
        })
        .await
        .into_iter()
        .collect(),
        None => HashMap::new(),
    };

    let devices = there
        .iter()
        .map(|address| Device {
            address: address.to_string(),
            name: names.remove(address).flatten(),
            mac: macs.remove(address),
            pinged: can_ping && answered.contains(address),
            ports: ports.remove(address).unwrap_or_default(),
        })
        .collect();
    Discovery {
        started_at,
        finished_at: Utc::now(),
        networks: ranges.iter().map(|r| r.network.clone()).collect(),
        scanned: targets.len() as u32,
        devices,
        note: (!notes.is_empty()).then(|| notes.join(". ")),
    }
    .bounded()
}

/// What to look at, of the networks the machine is on, and what to tell
/// people about it. One range per network, the smallest first, within
/// [`MAX_ADDRESSES`] in all.
fn plan(own: &[Own]) -> (Vec<Range>, Vec<String>) {
    let mut notes = Vec::new();
    let mut usable: Vec<&Own> = own
        .iter()
        .filter(|(name, address, prefix)| {
            let virtual_one = VIRTUAL
                .iter()
                .any(|start| name.to_lowercase().starts_with(start));
            is_private(*address) && *prefix < 31 && !virtual_one
        })
        .collect();
    // The longest prefix first: a small network is looked at whole before a
    // large one takes the room.
    usable.sort_by_key(|(_, address, prefix)| (std::cmp::Reverse(*prefix), *address));

    let mut ranges: Vec<Range> = Vec::new();
    let mut room = MAX_ADDRESSES;
    for (_, address, prefix) in usable {
        let Some(found) = range(*address, *prefix, room) else {
            notes.push(format!(
                "The network of {address} wasn't looked at: a sweep stops at {MAX_ADDRESSES} addresses"
            ));
            continue;
        };
        // Two addresses of one network are one network.
        if ranges.iter().any(|r| r.network == found.network) {
            continue;
        }
        if found.partial {
            notes.push(format!(
                "The network of {address}/{prefix} is larger than a sweep looks at: only {} addresses of {} were tried",
                found.addresses.len(),
                found.network
            ));
        }
        room -= found.addresses.len() as u32;
        ranges.push(found);
    }
    if ranges.is_empty() {
        notes.insert(
            0,
            "This machine is on no private network (10.x, 172.16 to 172.31.x, 192.168.x): there is nothing a sweep looks at"
                .to_string(),
        );
    }
    (ranges, notes)
}

/// The IPv4 addresses of the machine's interfaces, with their networks.
fn own_networks() -> Vec<Own> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    networks
        .iter()
        .flat_map(|(name, data)| {
            data.ip_networks().iter().filter_map(move |network| {
                let IpAddr::V4(address) = network.addr else {
                    return None;
                };
                Some((name.clone(), address, network.prefix))
            })
        })
        .collect()
}

/// `ask` of every address, [`AT_ONCE`] at a time.
async fn each<T, F, Fut>(addresses: &[Ipv4Addr], ask: F) -> Vec<(Ipv4Addr, T)>
where
    T: Send + 'static,
    F: Fn(Ipv4Addr) -> Fut,
    Fut: Future<Output = T> + Send + 'static,
{
    let gate = Arc::new(Semaphore::new(AT_ONCE));
    let mut tasks = JoinSet::new();
    for address in addresses.iter().copied() {
        let gate = gate.clone();
        let asked = ask(address);
        tasks.spawn(async move {
            let _slot = gate.acquire_owned().await;
            (address, asked.await)
        });
    }
    let mut answers = Vec::with_capacity(addresses.len());
    while let Some(joined) = tasks.join_next().await {
        if let Ok(answer) = joined {
            answers.push(answer);
        }
    }
    answers
}

/// Does `address` answer an echo?
async fn ping(monitor: &Monitor, address: Ipv4Addr) -> bool {
    let ip = IpAddr::V4(address);
    let Some(client) = monitor.icmp_client(ip) else {
        return false;
    };
    let mut pinger = client.pinger(ip, PingIdentifier(rand::random())).await;
    pinger.timeout(ECHO_TIMEOUT);
    for sequence in 0..ECHOES {
        if pinger.ping(PingSequence(sequence), &PAYLOAD).await.is_ok() {
            return true;
        }
    }
    false
}

/// What a TCP connection to a port came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Knock {
    /// Something listens.
    Open,
    /// Nothing listens, but the device said so: it is there.
    Refused,
    /// No answer in time.
    Silent,
}

async fn knock(address: Ipv4Addr, port: u16) -> Knock {
    let target = SocketAddr::from((address, port));
    match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(target)).await {
        Ok(Ok(_)) => Knock::Open,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Knock::Refused,
        _ => Knock::Silent,
    }
}

/// Which of [`PORTS`] accept a connection on `address`.
async fn open_ports(address: Ipv4Addr) -> Vec<u16> {
    let mut tasks = JoinSet::new();
    for (port, _) in PORTS {
        tasks.spawn(async move { (port, knock(address, port).await) });
    }
    let mut open = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok((port, Knock::Open)) = joined {
            open.push(port);
        }
    }
    open.sort_unstable();
    open
}

/// The name the network's DNS has for `address`, if any.
async fn name_of(resolver: &hickory_resolver::TokioResolver, address: Ipv4Addr) -> Option<String> {
    let [a, b, c, d] = address.octets();
    let reverse = format!("{d}.{c}.{b}.{a}.in-addr.arpa.");
    let asked = tokio::time::timeout(NAME_TIMEOUT, resolver.lookup(reverse, RecordType::PTR));
    let lookup = asked.await.ok()?.ok()?;
    lookup
        .answers()
        .iter()
        .find(|record| record.record_type() == RecordType::PTR)
        .map(|record| record.data.to_string().trim_end_matches('.').to_string())
        .filter(|name| !name.is_empty())
}

/// The hardware addresses in Linux's neighbour table (`/proc/net/arp`):
/// `IP address  HW type  Flags  HW address  Mask  Device`. An entry the
/// machine got no answer for has flags `0x0`.
#[cfg(any(target_os = "linux", test))]
fn from_arp_table(table: &str) -> Vec<(Ipv4Addr, String)> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let address: Ipv4Addr = fields.next()?.parse().ok()?;
            let (_, flags, mac) = (fields.next()?, fields.next()?, fields.next()?);
            let complete = u32::from_str_radix(flags.trim_start_matches("0x"), 16).ok()? & 0x2 != 0;
            complete.then_some(())?;
            Some((address, common::discovery::mac(mac)?))
        })
        .collect()
}

#[cfg(target_os = "linux")]
mod platform {
    use std::net::Ipv4Addr;

    /// The devices this machine knows the hardware address of.
    pub(super) async fn neighbours() -> Vec<(Ipv4Addr, String)> {
        match tokio::fs::read_to_string("/proc/net/arp").await {
            Ok(table) => super::from_arp_table(&table),
            Err(e) => {
                tracing::debug!("Can't read the neighbour table: {e}");
                Vec::new()
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::net::Ipv4Addr;

    use crate::checks::wmi::{self, Namespace};

    /// Windows' neighbour table, as `Get-NetNeighbor` reads it. A state of
    /// 0 or 1 (unreachable, incomplete) has no address worth showing.
    pub(super) async fn neighbours() -> Vec<(Ipv4Addr, String)> {
        let read = wmi::read(|| {
            let rows = Namespace::open(r"ROOT\StandardCimv2")?.rows(
                "MSFT_NetNeighbor",
                &["IPAddress", "LinkLayerAddress", "State"],
            )?;
            windows::core::Result::Ok(
                rows.into_iter()
                    .filter_map(|row| {
                        let state: u32 = row.get("State")?.parse().ok()?;
                        (state > 1).then_some(())?;
                        let address: Ipv4Addr = row.get("IPAddress")?.parse().ok()?;
                        let mac = common::discovery::mac(row.get("LinkLayerAddress")?)?;
                        Some((address, mac))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .await;
        match read {
            Some(Ok(neighbours)) => neighbours,
            Some(Err(e)) => {
                tracing::debug!("Can't read the neighbour table: {e}");
                Vec::new()
            }
            None => Vec::new(),
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    pub(super) async fn neighbours() -> Vec<(std::net::Ipv4Addr, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn own(name: &str, address: &str, prefix: u8) -> Own {
        (name.to_string(), address.parse().unwrap(), prefix)
    }

    #[test]
    fn only_the_machines_private_networks_are_planned() {
        let (ranges, notes) = plan(&[
            own("lo", "127.0.0.1", 8),
            own("eth0", "192.168.1.10", 24),
            own("eth0", "192.168.1.11", 24),
            own("wan0", "203.0.113.7", 24),
            own("docker0", "172.17.0.1", 16),
            own("vEthernet (WSL)", "172.28.0.1", 20),
            own("tun0", "10.8.0.2", 32),
        ]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].network, "192.168.1.0/24");
        assert_eq!(ranges[0].addresses.len(), 253);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_machine_on_no_private_network_looks_at_nothing_and_says_so() {
        let (ranges, notes) = plan(&[own("lo", "127.0.0.1", 8), own("eth0", "203.0.113.7", 24)]);
        assert!(ranges.is_empty());
        assert!(
            notes[0].starts_with("This machine is on no private network"),
            "{notes:?}"
        );
    }

    #[test]
    fn a_sweep_stays_within_its_room_the_small_networks_first() {
        let (ranges, notes) = plan(&[
            own("eth0", "10.20.30.40", 8),
            own("eth1", "192.168.5.9", 24),
            own("eth2", "172.16.0.5", 22),
        ]);
        let named: Vec<&str> = ranges.iter().map(|r| r.network.as_str()).collect();
        // The /24 whole; the /22 no longer fits, and the /8 never does:
        // of those, the /24 around the machine.
        assert_eq!(named, ["192.168.5.0/24", "172.16.0.0/24", "10.20.30.0/24"]);
        let total: usize = ranges.iter().map(|r| r.addresses.len()).sum();
        assert_eq!(total, 3 * 253);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("172.16.0.5/22"), "{notes:?}");
        assert!(notes[1].contains("10.20.30.40/8"), "{notes:?}");

        // Five networks: the fifth finds the room nearly gone, the sixth none.
        let many: Vec<Own> = (0..6u8)
            .map(|n| own("eth", &format!("192.168.{n}.9"), 24))
            .collect();
        let (ranges, notes) = plan(&many);
        assert_eq!(ranges.len(), 5);
        let total: usize = ranges.iter().map(|r| r.addresses.len()).sum();
        assert_eq!(total, MAX_ADDRESSES as usize);
        assert!(notes[1].contains("wasn't looked at"), "{notes:?}");

        let (ranges, notes) = plan(&[own("eth0", "10.20.30.40", 8)]);
        assert_eq!(ranges[0].network, "10.20.30.0/24");
        assert!(
            notes[0].contains("only 253 addresses of 10.20.30.0/24 were tried"),
            "{notes:?}"
        );
    }

    #[test]
    fn linuxs_neighbour_table_gives_the_hardware_addresses_it_learned() {
        let table = "IP address       HW type     Flags       HW address            Mask     Device\n\
                     192.168.1.1      0x1         0x2         AA:BB:CC:00:11:22     *        eth0\n\
                     192.168.1.7      0x1         0x0         00:00:00:00:00:00     *        eth0\n\
                     192.168.1.9      0x1         0x6         00:11:32:aa:bb:cc     *        eth0\n\
                     garbage\n";
        assert_eq!(
            from_arp_table(table),
            [
                (
                    "192.168.1.1".parse().unwrap(),
                    "aa:bb:cc:00:11:22".to_string()
                ),
                (
                    "192.168.1.9".parse().unwrap(),
                    "00:11:32:aa:bb:cc".to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_port_that_listens_is_open_and_one_that_doesnt_refuses() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open = listener.local_addr().unwrap().port();
        assert_eq!(knock(Ipv4Addr::LOCALHOST, open).await, Knock::Open);
        drop(listener);
        // Nothing listens there now: this machine says so.
        // (Windows takes longer than a sweep waits to say so: silent there.)
        assert_ne!(knock(Ipv4Addr::LOCALHOST, open).await, Knock::Open);
    }

    #[tokio::test]
    async fn every_address_is_asked_and_answered_for() {
        let addresses: Vec<Ipv4Addr> = (1..=200u8).map(|n| Ipv4Addr::new(10, 0, 0, n)).collect();
        let mut answers = each(
            &addresses,
            |address| async move { address.octets()[3] % 2 == 0 },
        )
        .await;
        answers.sort();
        assert_eq!(answers.len(), 200);
        assert_eq!(answers[0], (Ipv4Addr::new(10, 0, 0, 1), false));
        assert_eq!(answers[1], (Ipv4Addr::new(10, 0, 0, 2), true));
    }

    /// The real thing, on the network of the machine the test runs on:
    /// `cargo test -p probe this_machines_network -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "sweeps the network of the machine it runs on"]
    async fn this_machines_network() {
        let found = sweep(Arc::new(Monitor::new())).await;
        println!(
            "{} of {} addresses in {:?} answered, in {} s. {}",
            found.devices.len(),
            found.scanned,
            found.networks,
            (found.finished_at - found.started_at).num_seconds(),
            found.note.as_deref().unwrap_or("")
        );
        for device in &found.devices {
            println!(
                "{:<16} {:<18} {:<5} {:<30} {:?}",
                device.address,
                device.mac.as_deref().unwrap_or("-"),
                if device.pinged { "ping" } else { "-" },
                device.name.as_deref().unwrap_or("-"),
                device.ports
            );
        }
    }
}
