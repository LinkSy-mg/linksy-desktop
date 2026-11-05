//! The Connection Doctor: "Why can't I see my phone?". It looks at what is wrong from this
//! computer's side and says it in a sentence, with a button where Linksy can act. The firewall
//! is the apps' to look at (Finish Setup's check needs the GUI crates the service doesn't carry).
//!
//! What it can tell, and how:
//! - a VPN: the default route goes out through a VPN's interface;
//! - a phone on another network: its last address is outside every network this computer is on;
//! - a phone that doesn't answer: its last address is on this network, and a datagram sent to
//!   it gets no answer to the ARP request that it needs (Wi-Fi off, or a router that keeps
//!   devices apart);
//! - a phone that answers but whose Linksy doesn't: the same, with an ARP answer;
//! - the cable: a phone plugged in whose cable isn't on, or didn't connect.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use linksy_core::{DeviceKind, Node};
use linksy_i18n::t;
use linksy_ipc::{Finding, FindingAction, UsbPhone, UsbState};

/// What an address's neighbour entry says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arp {
    /// Not looked at (not on a network of this computer's).
    Unknown,
    /// Something at that address answered.
    Answers,
    /// Nothing did.
    Silent,
}

/// A paired phone that isn't connected.
pub struct Phone {
    pub name: String,
    pub last: Option<SocketAddr>,
    pub arp: Arp,
}

pub struct Facts {
    /// The interface of a VPN that carries the default route.
    pub vpn: Option<String>,
    /// This computer's IPv4 networks: an address and its prefix length.
    pub networks: Vec<(Ipv4Addr, u8)>,
    pub phones: Vec<Phone>,
    pub cables: Vec<UsbPhone>,
}

fn finding(id: &str, title: impl Into<String>, detail: impl Into<String>, action: Option<FindingAction>) -> Finding {
    Finding { id: id.into(), title: title.into(), detail: detail.into(), action }
}

pub fn diagnose(f: &Facts) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(iface) = &f.vpn {
        out.push(finding(
            "vpn",
            t!("A VPN is carrying your traffic"),
            t!("{iface} carries this computer's traffic, which can hide devices on your Wi-Fi. Allow local network access in the VPN, or pause it.", iface),
            None,
        ));
    }
    for p in &f.phones {
        let name = p.name.clone();
        let Some(last) = p.last else {
            out.push(finding(
                "never_seen",
                t!("{name} hasn't been seen on a network yet", name),
                t!("Put both on the same Wi-Fi, or connect by the address shown in Linksy on the phone."),
                Some(FindingAction::ConnectByAddress { address: String::new() }),
            ));
            continue;
        };
        // Last reached over the cable or the relay: it came through this computer's own bridge
        let IpAddr::V4(ip) = last.ip() else { continue };
        if ip.is_loopback() {
            continue;
        }
        let address = last.ip().to_string();
        let connect = Some(FindingAction::ConnectByAddress { address: address.clone() });
        if !on_a_network(ip, &f.networks) {
            let networks = f.networks.iter().map(|(n, p)| format!("{}/{p}", network_of(*n, *p))).collect::<Vec<_>>().join(", ");
            out.push(finding(
                "other_network",
                t!("{name} is on another network", name),
                t!("Last seen at {address}, outside this computer's network ({networks}). Devices on different networks, such as a guest network or an extender's, can't find each other.", address, networks),
                connect,
            ));
            continue;
        }
        match p.arp {
            Arp::Silent => out.push(finding(
                "silent",
                t!("{name} doesn't answer", name),
                t!("Last seen at {address}, where nothing answers now. Its Wi-Fi may be off, or the router may keep devices apart (a guest network, AP isolation).", address),
                connect,
            )),
            Arp::Answers => out.push(finding(
                "asleep",
                t!("{name} answers, but Linksy there doesn't", name),
                t!("Open Linksy on the phone. If this keeps happening, turn on Keep Linksy alive in its settings so Android doesn't stop it."),
                connect,
            )),
            Arp::Unknown => {}
        }
    }
    for c in &f.cables {
        let name = c.name.clone();
        let phone = FindingAction::UsbConnect { phone: c.key.clone() };
        match c.state {
            UsbState::Plugged | UsbState::Declined => out.push(finding(
                "cable_off",
                t!("{name} is plugged in, but the cable isn't on", name),
                t!("Switch the cable on and Linksy connects through it, with no Wi-Fi."),
                Some(phone),
            )),
            UsbState::Failed => out.push(finding(
                "cable_failed",
                t!("The cable to {name} didn't connect", name),
                t!("Try again, or unplug it and plug it back in. {reason}", reason = c.error.clone().unwrap_or_default()),
                Some(phone),
            )),
            UsbState::Switching | UsbState::Accessory => {}
        }
    }
    if out.is_empty() {
        out.push(finding(
            "fine",
            t!("Nothing looks wrong"),
            t!("The firewall, the network and the cable look fine from here. On the phone, open Linksy and check that both are on the same Wi-Fi."),
            None,
        ));
    }
    out
}

fn on_a_network(ip: Ipv4Addr, networks: &[(Ipv4Addr, u8)]) -> bool {
    networks.iter().any(|(n, p)| network_of(*n, *p) == network_of(ip, *p))
}

fn network_of(ip: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix.min(32))) };
    Ipv4Addr::from(u32::from(ip) & mask)
}

/// The interface of the default route with the lowest metric, from /proc/net/route.
pub fn default_interface(route: &str) -> Option<String> {
    route
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let up = u32::from_str_radix(f.get(3)?, 16).ok()? & 0x1 != 0;
            (up && *f.get(1)? == "00000000" && *f.get(7)? == "00000000").then(|| Some((f.get(6)?.parse::<u32>().ok()?, f[0].to_string())))?
        })
        .min()
        .map(|(_, iface)| iface)
}

/// Whether an interface's name is a VPN's.
pub fn is_vpn(iface: &str) -> bool {
    ["tun", "wg", "ppp", "tailscale", "zt", "nordlynx", "proton", "mullvad", "vpn", "ipsec"].iter().any(|p| iface.starts_with(p))
}

/// What /proc/net/arp says of an address: a complete entry answers, anything else doesn't.
pub fn arp_of(table: &str, ip: Ipv4Addr) -> Arp {
    let entry = table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.first()? == &ip.to_string().as_str()).then(|| u32::from_str_radix(f.get(2)?.trim_start_matches("0x"), 16).ok())?
    });
    match entry {
        Some(flags) if flags & 0x2 != 0 => Arp::Answers,
        _ => Arp::Silent,
    }
}

/// Container and VM bridges: devices on them are never phones.
const VIRTUAL: [&str; 10] = ["docker", "br-", "veth", "virbr", "lxc", "lxd", "podman", "cni", "vmnet", "vboxnet"];

/// Look at everything, off the async runtime: files under /proc, and a short wait for the ARP
/// answers that a datagram to each address on this network provokes.
fn gather(paired: Vec<(String, Option<SocketAddr>)>, cables: Vec<UsbPhone>) -> Facts {
    let route = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let vpn = default_interface(&route).filter(|i| is_vpn(i));
    let networks: Vec<(Ipv4Addr, u8)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && !VIRTUAL.iter().any(|v| i.name.starts_with(v)))
        .filter_map(|i| match i.addr {
            // A single address (a VPN's) isn't a network anyone else is on
            if_addrs::IfAddr::V4(v4) => Some((v4.ip, u32::from(v4.netmask).count_ones() as u8)).filter(|(_, p)| *p < 31),
            _ => None,
        })
        .collect();
    let near: Vec<&SocketAddr> = paired.iter().filter_map(|(_, a)| a.as_ref()).filter(|a| matches!(a.ip(), IpAddr::V4(ip) if on_a_network(ip, &networks))).collect();
    if let (false, Ok(socket)) = (near.is_empty(), std::net::UdpSocket::bind("0.0.0.0:0")) {
        for a in &near {
            let _ = socket.send_to(&[0], a);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let table = std::fs::read_to_string("/proc/net/arp").unwrap_or_default();
    let phones = paired
        .into_iter()
        .map(|(name, last)| {
            let arp = match last.map(|a| a.ip()) {
                Some(IpAddr::V4(ip)) if on_a_network(ip, &networks) => arp_of(&table, ip),
                _ => Arp::Unknown,
            };
            Phone { name, last, arp }
        })
        .collect();
    Facts { vpn, networks, phones, cables }
}

/// The findings for this computer and its paired phones that aren't connected.
pub async fn run(node: &Node, cables: Vec<UsbPhone>) -> Vec<Finding> {
    let paired: Vec<(String, Option<SocketAddr>)> = node
        .devices()
        .into_iter()
        .filter(|d| d.trusted && !d.connected && matches!(d.device.kind, DeviceKind::Phone | DeviceKind::Tablet))
        .map(|d| (d.device.name.clone(), node.last_address(&d.device.id)))
        .collect();
    let facts = tokio::task::spawn_blocking(move || gather(paired, cables)).await.unwrap_or_else(|_| Facts { vpn: None, networks: Vec::new(), phones: Vec::new(), cables: Vec::new() });
    diagnose(&facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net() -> Vec<(Ipv4Addr, u8)> {
        vec![(Ipv4Addr::new(192, 168, 1, 20), 24)]
    }

    fn phone(last: Option<&str>, arp: Arp) -> Phone {
        Phone { name: "Pixel".into(), last: last.map(|a| a.parse().unwrap()), arp }
    }

    fn facts(phones: Vec<Phone>) -> Facts {
        Facts { vpn: None, networks: net(), phones, cables: Vec::new() }
    }

    fn ids(f: &Facts) -> Vec<String> {
        diagnose(f).into_iter().map(|f| f.id).collect()
    }

    #[test]
    fn nothing_wrong_says_so() {
        assert_eq!(ids(&facts(Vec::new())), ["fine"]);
    }

    #[test]
    fn a_phone_on_another_subnet_is_named_with_its_address() {
        let f = diagnose(&facts(vec![phone(Some("192.168.2.40:47200"), Arp::Unknown)]));
        assert_eq!(f[0].id, "other_network");
        assert!(f[0].detail.contains("192.168.2.40") && f[0].detail.contains("192.168.1.0/24"), "{}", f[0].detail);
        assert_eq!(f[0].action, Some(FindingAction::ConnectByAddress { address: "192.168.2.40".into() }));
    }

    #[test]
    fn a_phone_on_this_network_that_is_silent_or_answers() {
        assert_eq!(ids(&facts(vec![phone(Some("192.168.1.40:47200"), Arp::Silent)])), ["silent"]);
        assert_eq!(ids(&facts(vec![phone(Some("192.168.1.40:47200"), Arp::Answers)])), ["asleep"]);
    }

    #[test]
    fn a_phone_last_reached_over_the_cable_or_the_relay_is_not_on_another_network() {
        assert_eq!(ids(&facts(vec![phone(Some("127.0.0.1:41000"), Arp::Unknown), phone(Some("127.0.0.2:1"), Arp::Unknown)])), ["fine"]);
    }

    #[test]
    fn a_phone_never_seen_is_offered_the_address() {
        let f = diagnose(&facts(vec![phone(None, Arp::Unknown)]));
        assert_eq!(f[0].id, "never_seen");
        assert_eq!(f[0].action, Some(FindingAction::ConnectByAddress { address: String::new() }));
    }

    #[test]
    fn a_vpn_is_reported_beside_the_rest() {
        let mut f = facts(vec![phone(Some("192.168.1.40:47200"), Arp::Silent)]);
        f.vpn = Some("wg0".into());
        assert_eq!(ids(&f), ["vpn", "silent"]);
    }

    #[test]
    fn the_cable_is_offered_when_it_is_off_or_failed() {
        let cable = |state, error: Option<&str>| UsbPhone { key: "abc".into(), name: "Pixel".into(), state, error: error.map(Into::into), auto: false };
        let mut f = facts(Vec::new());
        f.cables = vec![cable(UsbState::Plugged, None)];
        let found = diagnose(&f);
        assert_eq!((found[0].id.as_str(), &found[0].action), ("cable_off", &Some(FindingAction::UsbConnect { phone: "abc".into() })));
        f.cables = vec![cable(UsbState::Failed, Some("no answer"))];
        let found = diagnose(&f);
        assert_eq!(found[0].id, "cable_failed");
        assert!(found[0].detail.contains("no answer"));
        f.cables = vec![cable(UsbState::Accessory, None)];
        assert_eq!(ids(&f), ["fine"]);
    }

    #[test]
    fn the_default_route_is_the_one_with_the_lowest_metric() {
        let route = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
                     wlp3s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                     wg0\t00000000\t0100000A\t0003\t0\t0\t50\t00000000\t0\t0\t0\n\
                     wlp3s0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(default_interface(route).as_deref(), Some("wg0"));
        assert!(is_vpn("wg0") && is_vpn("tun0") && is_vpn("tailscale0") && !is_vpn("wlp3s0") && !is_vpn("enp0s31f6"));
    }

    #[test]
    fn a_complete_neighbour_entry_answers_and_anything_else_does_not() {
        let table = "IP address       HW type     Flags       HW address            Mask     Device\n\
                     192.168.1.40     0x1         0x2         aa:bb:cc:dd:ee:ff     *        wlp3s0\n\
                     192.168.1.41     0x1         0x0         00:00:00:00:00:00     *        wlp3s0\n";
        assert_eq!(arp_of(table, Ipv4Addr::new(192, 168, 1, 40)), Arp::Answers);
        assert_eq!(arp_of(table, Ipv4Addr::new(192, 168, 1, 41)), Arp::Silent);
        assert_eq!(arp_of(table, Ipv4Addr::new(192, 168, 1, 42)), Arp::Silent);
    }

    #[test]
    fn networks_are_compared_by_their_prefix() {
        let n = vec![(Ipv4Addr::new(10, 0, 5, 7), 16)];
        assert!(on_a_network(Ipv4Addr::new(10, 0, 200, 1), &n));
        assert!(!on_a_network(Ipv4Addr::new(10, 1, 0, 1), &n));
    }
}
