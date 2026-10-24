//! Joining a phone's direct link (docs/protocol.md §4.17), its Wi-Fi Direct group, through
//! NetworkManager: no root needed, and this computer stays on its own Wi-Fi network
//! meanwhile where the card can do both at once (most can).
//!
//! The phone finds this computer and connects to it, then this side connects back:
//! both asking is what spares the phone an "invitation to connect" to accept. Nothing runs
//! between links; each one has its own system bus connection, and NetworkManager drops the
//! link by itself if this service goes away.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use linksy_core::{DirectStep, Event, Fingerprint, Node};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tracing::info;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const P2P: &str = "org.freedesktop.NetworkManager.Device.WifiP2P";
const PEER: &str = "org.freedesktop.NetworkManager.WifiP2PPeer";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
/// What Android gives apps instead of their own Wi-Fi Direct address.
const HIDDEN: &str = "02:00:00:00:00:00";
/// NetworkManager's type for a Wi-Fi Direct device (`p2p-dev-wlan0`).
const WIFI_P2P: u32 = 30;
/// A connection's states: activating, activated.
const ACTIVATING: u32 = 1;
const ACTIVATED: u32 = 2;

/// Whether this computer can join direct links: NetworkManager has a Wi-Fi Direct device
/// (the Wi-Fi card and its driver can do it) and lets this service talk to it.
pub async fn available() -> bool {
    match zbus::Connection::system().await {
        Ok(bus) => p2p_device(&bus).await.is_some(),
        Err(_) => false,
    }
}

pub async fn run(node: Node) {
    let mut events = node.subscribe();
    // Each link being joined or up, and where its next steps go.
    let mut links: HashMap<Fingerprint, mpsc::UnboundedSender<DirectStep>> = HashMap::new();
    loop {
        match events.recv().await {
            Ok(Event::Direct { from, step: DirectStep::Invite { address, name } }) => {
                let (steps, rx) = mpsc::unbounded_channel();
                // Replaces one still going, which ends with its channel
                links.insert(from.id, steps);
                // Its Wi-Fi Direct name is usually its device name too
                let names = name.into_iter().chain([from.name.clone()]).collect();
                tokio::spawn(join(node.clone(), from.id, Host { address, names }, rx));
            }
            Ok(Event::Direct { from, step }) => {
                if let Some(steps) = links.get(&from.id) {
                    let _ = steps.send(step);
                }
            }
            Ok(Event::DirectEnded { id, .. }) => {
                links.remove(&id);
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

/// The phone hosting a link, as it said: its Wi-Fi Direct address (hidden on Android) and
/// the names it may go by.
struct Host {
    address: String,
    names: Vec<String>,
}

/// What NetworkManager holds for a link, let go of when it ends.
#[derive(Default)]
struct Joined {
    bus: Option<zbus::Connection>,
    device: Option<OwnedObjectPath>,
    active: Option<OwnedObjectPath>,
}

async fn join(node: Node, id: Fingerprint, host: Host, mut steps: mpsc::UnboundedReceiver<DirectStep>) {
    let mut joined = Joined::default();
    if let Err(e) = joining(&node, &id, &host, &mut steps, &mut joined).await {
        info!("no direct link: {e}");
        node.direct_end(&id, Some(e)).await;
    }
    let Some(bus) = &joined.bus else { return };
    if let Some(active) = &joined.active {
        let _ = bus.call_method(Some(NM), NM_PATH, Some(NM), "DeactivateConnection", active).await;
    }
    // Searching takes the Wi-Fi's time from the usual network
    if let Some(device) = &joined.device {
        let _ = bus.call_method(Some(NM), device, Some(P2P), "StopFind", &()).await;
    }
}

/// Become findable, connect back once the phone connects, and open Linksy's connection
/// over the link once it's up and the phone said where it is. Returns when the link ends.
async fn joining(node: &Node, id: &Fingerprint, host: &Host, steps: &mut mpsc::UnboundedReceiver<DirectStep>, joined: &mut Joined) -> Result<(), String> {
    let bus = zbus::Connection::system().await.map_err(|e| format!("no system bus: {e}"))?;
    let (device, interface) = p2p_device(&bus).await.ok_or("this computer has no Wi-Fi Direct")?;
    let me = own_address(&interface).await.ok_or("this computer's Wi-Fi Direct address is unknown")?;
    let find = HashMap::from([("timeout", Value::from(30i32))]);
    bus.call_method(Some(NM), &device, Some(P2P), "StartFind", &find).await.map_err(|e| format!("Wi-Fi Direct search: {e}"))?;
    joined.bus = Some(bus.clone());
    joined.device = Some(device.clone());
    info!("direct link: findable as {me}");
    node.direct_send(id, DirectStep::Listening { address: me }).await.map_err(|e| e.to_string())?;

    let mut up = None;
    let mut ready = false;
    let mut dialed = false;
    let mut check = tokio::time::interval(Duration::from_millis(300));
    loop {
        tokio::select! {
            step = steps.recv() => match step {
                None => return Ok(()),
                Some(DirectStep::Connecting) if joined.active.is_none() => {
                    info!("direct link: the phone is connecting");
                    let peer = find_peer(&bus, &device, host).await.ok_or("the phone didn't show up over Wi-Fi Direct")?;
                    info!("direct link: connecting to {peer}");
                    joined.active = Some(activate(&bus, &device, &peer).await?);
                }
                Some(DirectStep::Up { ip, port }) => {
                    info!("direct link: the phone is at {ip}:{port}");
                    up = Some(SocketAddr::new(ip, port));
                }
                Some(_) => {}
            },
            _ = check.tick(), if joined.active.is_some() => {
                let state = property::<u32>(&bus, joined.active.as_ref().expect("checked"), ACTIVE, "State").await.ok();
                match state {
                    Some(ACTIVATED) if !ready => {
                        info!("direct link: Wi-Fi Direct connected");
                        ready = true;
                        // Up: only to notice it dropping
                        check = tokio::time::interval(Duration::from_secs(2));
                    }
                    Some(ACTIVATED) | Some(ACTIVATING) | Some(0) => {}
                    _ if ready => return Err("the Wi-Fi Direct link dropped".into()),
                    _ => return Err("the Wi-Fi Direct connection failed".into()),
                }
            }
        }
        if ready && !dialed && let Some(addr) = up {
            node.direct_dial(id, addr).await.map_err(|e| format!("Linksy didn't answer over the link: {e}"))?;
            dialed = true;
        }
    }
}

/// Connect to the phone's Wi-Fi Direct device (push-button: it asked too).
async fn activate(bus: &zbus::Connection, device: &OwnedObjectPath, peer: &str) -> Result<OwnedObjectPath, String> {
    let mut connection = HashMap::from([("id", Value::from("Linksy direct link")), ("type", Value::from("wifi-p2p")), ("autoconnect", Value::from(false))]);
    // The user's own connection, which needs no administrator password
    if let Ok(user) = std::env::var("USER") {
        connection.insert("permissions", Value::from(vec![format!("user:{user}")]));
    }
    let settings: HashMap<&str, HashMap<&str, Value>> = HashMap::from([
        ("connection", connection),
        ("wifi-p2p", HashMap::from([("peer", Value::from(peer))])),
        // The internet stays on the usual network
        ("ipv4", HashMap::from([("method", Value::from("auto")), ("never-default", Value::from(true)), ("ignore-auto-dns", Value::from(true))])),
        ("ipv6", HashMap::from([("method", Value::from("ignore"))])),
    ]);
    // Never saved, and dropped if this service goes away
    let options = HashMap::from([("persist", Value::from("volatile")), ("bind-activation", Value::from("dbus-client"))]);
    let none = ObjectPath::try_from("/").expect("valid path");
    let reply = bus
        .call_method(Some(NM), NM_PATH, Some(NM), "AddAndActivateConnection2", &(settings, device, none, options))
        .await
        .map_err(|e| format!("Wi-Fi Direct connection: {e}"))?;
    let (_, active, _): (OwnedObjectPath, OwnedObjectPath, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| e.to_string())?;
    Ok(active)
}

/// The host among the Wi-Fi Direct devices around, waiting a few seconds for it: by its
/// address, or by its name where it can't know its address (Android), or didn't say the one
/// it shows. Not by a name two devices share: that would ask the wrong one.
async fn find_peer(bus: &zbus::Connection, device: &OwnedObjectPath, host: &Host) -> Option<String> {
    let hidden = host.address == HIDDEN;
    for tries in 0..20 {
        let peers: Vec<OwnedObjectPath> = property(bus, device, P2P, "Peers").await.unwrap_or_default();
        let mut named = Vec::new();
        for peer in &peers {
            let Ok(address) = property::<String>(bus, peer, PEER, "HwAddress").await else { continue };
            if address.eq_ignore_ascii_case(&host.address) {
                return Some(address);
            }
            let name = property::<String>(bus, peer, PEER, "Name").await.unwrap_or_default();
            if !name.trim().is_empty() && host.names.iter().any(|n| n.trim().eq_ignore_ascii_case(name.trim())) {
                named.push(address);
            }
        }
        if named.len() == 1 && (hidden || tries >= 6) {
            return named.pop();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}

/// NetworkManager's Wi-Fi Direct device and its name (`p2p-dev-wlan0`).
async fn p2p_device(bus: &zbus::Connection) -> Option<(OwnedObjectPath, String)> {
    let reply = bus.call_method(Some(NM), NM_PATH, Some(NM), "GetDevices", &()).await.ok()?;
    let devices: Vec<OwnedObjectPath> = reply.body().deserialize().ok()?;
    for device in devices {
        if property::<u32>(bus, &device, DEVICE, "DeviceType").await.ok() == Some(WIFI_P2P) {
            let name = property::<String>(bus, &device, DEVICE, "Interface").await.ok()?;
            return Some((device, name));
        }
    }
    None
}

async fn property<T>(bus: &zbus::Connection, path: &OwnedObjectPath, interface: &str, name: &str) -> zbus::Result<T>
where
    T: TryFrom<OwnedValue, Error = zbus::zvariant::Error>,
{
    let reply = bus.call_method(Some(NM), path, Some("org.freedesktop.DBus.Properties"), "Get", &(interface, name)).await?;
    let value: OwnedValue = reply.body().deserialize()?;
    Ok(T::try_from(value)?)
}

/// This computer's Wi-Fi Direct device address, which the phone looks for: as `iw` shows
/// it, or else the Wi-Fi card's own address, which the kernel gives that device when the
/// card has just one (the network then gets a random one, as NetworkManager does by default).
async fn own_address(interface: &str) -> Option<String> {
    let iw = tokio::process::Command::new("iw").arg("dev").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().await;
    if let Some(address) = iw.ok().filter(|o| o.status.success()).and_then(|o| p2p_address(&String::from_utf8_lossy(&o.stdout))) {
        return Some(address);
    }
    let wifi = interface.strip_prefix("p2p-dev-")?;
    let address = std::fs::read_to_string(format!("/sys/class/net/{wifi}/phy80211/macaddress")).ok()?;
    Some(address.trim().to_lowercase())
}

/// The Wi-Fi Direct device's address in what `iw dev` prints.
fn p2p_address(iw: &str) -> Option<String> {
    let mut address = None;
    for line in iw.lines().map(str::trim) {
        if let Some(a) = line.strip_prefix("addr ") {
            address = Some(a.to_lowercase());
        } else if line.starts_with("Interface ") || line.starts_with("Unnamed") || line.starts_with("phy#") {
            address = None;
        } else if line == "type P2P-device" {
            return address;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_wifi_direct_address_in_iw() {
        let iw = "phy#0\n\tUnnamed/non-netdev interface\n\t\twdev 0x2\n\t\taddr d8:80:83:cb:0d:33\n\t\ttype P2P-device\n\t\ttxpower 23.00 dBm\n\
                  \tInterface wlp3s0\n\t\tifindex 3\n\t\twdev 0x1\n\t\taddr 2a:9b:ed:2d:28:94\n\t\ttype managed\n";
        assert_eq!(p2p_address(iw).as_deref(), Some("d8:80:83:cb:0d:33"));
        assert_eq!(p2p_address("phy#0\n\tInterface wlan0\n\t\taddr 00:11:22:33:44:55\n\t\ttype managed\n"), None);
    }

    /// Asks NetworkManager for this computer's Wi-Fi Direct device. Needs one.
    #[tokio::test]
    #[ignore = "needs NetworkManager and a Wi-Fi card that does Wi-Fi Direct"]
    async fn finds_this_computers_wifi_direct() {
        let bus = zbus::Connection::system().await.unwrap();
        let (device, interface) = p2p_device(&bus).await.expect("a Wi-Fi Direct device");
        let address = own_address(&interface).await.expect("its address");
        let _: Vec<OwnedObjectPath> = property(&bus, &device, P2P, "Peers").await.expect("the devices around");
        assert!(linksy_core::DirectStep::Listening { address }.is_valid());
    }

    /// Joins a phone that isn't there: NetworkManager takes it (no password asked), looks
    /// for it and gives up, and nothing stays behind. Needs a Wi-Fi Direct device.
    #[tokio::test]
    #[ignore = "needs NetworkManager and a Wi-Fi card that does Wi-Fi Direct"]
    async fn joining_nobody_fails_and_leaves_nothing() {
        let bus = zbus::Connection::system().await.unwrap();
        let (device, _) = p2p_device(&bus).await.expect("a Wi-Fi Direct device");
        let active = activate(&bus, &device, "02:00:00:00:00:01").await.unwrap();
        let mut last = None;
        for _ in 0..100 {
            last = property::<u32>(&bus, &active, ACTIVE, "State").await.ok();
            if !matches!(last, Some(0 | ACTIVATING)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        assert_ne!(last, Some(ACTIVATED));
        let _ = bus.call_method(Some(NM), NM_PATH, Some(NM), "DeactivateConnection", &active).await;
    }
}
