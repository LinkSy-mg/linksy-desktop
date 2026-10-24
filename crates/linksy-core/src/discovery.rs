//! Local network discovery over mDNS/DNS-SD (`_linksy._udp.local.`), private
//! (docs/protocol.md §4.16): a device announces only a tag that changes every 15 minutes,
//! made from its discovery key, which only its paired devices know. Its id, name and OS go
//! on the network only while it is being added (pairing mode), or while a paired device
//! hasn't moved to private discovery yet (the §4.1 record). Identity is still proven by the
//! TLS handshake, never by a record.
//!
//! Presence: a device that leaves cleanly says goodbye. One that just goes to sleep or out
//! of range doesn't, so every visible device is re-queried once in a while (one small
//! multicast packet) and dropped when it no longer answers.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, SocketAddrV6};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mdns_sd::{ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, warn};

use crate::error::Result;
use crate::identity::{DeviceInfo, DeviceKind, Fingerprint};

pub const SERVICE_TYPE: &str = "_linksy._udp.local.";
/// How often visible devices are asked whether they are still there.
// Don't ask why 45. It works with 45.
const PRESENCE_CHECK_EVERY: Duration = Duration::from_secs(45);
/// A tag lasts one epoch.
pub const EPOCH_SECS: u64 = 900;
/// How often the announced records are checked against what they should be (a new epoch,
/// pairing mode over).
const ANNOUNCE_CHECK_EVERY: Duration = Duration::from_secs(10);

/// A device's discovery key: its tags are made from it.
pub type DiscoveryKey = [u8; 32];

/// This device's discovery key, made the first time.
pub(crate) fn load_or_create_key(dir: &Path) -> Result<DiscoveryKey> {
    let path = dir.join("discovery.key");
    if let Ok(bytes) = std::fs::read(&path)
        && let Ok(key) = <DiscoveryKey>::try_from(bytes.as_slice())
    {
        return Ok(key);
    }
    new_key(dir)
}

/// A new discovery key, replacing the old one (a forgotten device can't recognise this
/// one any more).
pub(crate) fn new_key(dir: &Path) -> Result<DiscoveryKey> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(std::io::Error::other)?;
    crate::identity::write_private(&dir.join("discovery.key"), &key)?;
    Ok(key)
}

pub fn epoch_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / EPOCH_SECS
}

/// The tag `key` announces during `epoch`: the first 16 bytes of
/// HMAC-SHA256(key, "linksy discovery v2" || u64_be(epoch)), as 32 lowercase hex chars.
pub fn tag(key: &DiscoveryKey, epoch: u64) -> String {
    let mut data = b"linksy discovery v2".to_vec();
    data.extend_from_slice(&epoch.to_be_bytes());
    hex::encode(&hmac_sha256(key, &data)[..16])
}

fn hmac_sha256(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for (i, k) in key.iter().enumerate() {
        ipad[i] ^= k;
        opad[i] ^= k;
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(data).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

pub(crate) enum DiscoveryEvent {
    Found { device: DeviceInfo, addrs: Vec<SocketAddr> },
    Lost { id: Fingerprint },
}

/// What this device announces, and whose tags it looks for. Set by the node.
#[derive(Clone)]
pub(crate) struct Announce {
    pub me: DeviceInfo,
    pub key: DiscoveryKey,
    /// Also the §4.1 record: a paired device doesn't speak §4.16 yet.
    pub legacy: bool,
    /// Findable by strangers until then (the invite dialog is open).
    pub pairing_until: Option<Instant>,
    /// Paired devices with the key they announce with.
    pub peers: Vec<(DeviceInfo, DiscoveryKey)>,
}

struct Shared {
    announce: Announce,
    /// Records registered now: fullname → their TXT properties.
    registered: HashMap<String, Vec<(String, String)>>,
    /// Device records seen: fullname → device id.
    names: HashMap<String, Fingerprint>,
    /// Private records seen: fullname → (tag, addresses), matched or not.
    tags: HashMap<String, (String, Vec<SocketAddr>)>,
}

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    shared: Arc<Mutex<Shared>>,
    wake: Arc<Notify>,
    tx: mpsc::UnboundedSender<DiscoveryEvent>,
}

fn props_of(me: &DeviceInfo, v: &str, pairing: bool) -> Vec<(String, String)> {
    let kind = serde_json::to_value(me.kind).ok().and_then(|k| k.as_str().map(String::from)).unwrap_or_else(|| "desktop".into());
    let mut props = vec![("id".into(), me.id.to_string()), ("name".into(), me.name.clone()), ("kind".into(), kind), ("os".into(), me.os.clone()), ("v".into(), v.into())];
    if pairing {
        props.push(("p".into(), "1".into()));
    }
    props
}

/// One record to announce.
struct Record {
    instance: String,
    host: String,
    props: Vec<(String, String)>,
}

/// The records `a` should be announced as now.
fn wanted(a: &Announce, now: Instant) -> Vec<Record> {
    let t = tag(&a.key, epoch_now());
    let mut out = vec![Record { instance: format!("Linksy {}", &t[..8]), host: format!("linksy-{}.local.", &t[..8]), props: vec![("v".into(), "2".into()), ("t".into(), t)] }];
    let pairing = a.pairing_until.is_some_and(|u| now < u);
    // One named record at most: the §4.1 one already lists it to everyone.
    if a.legacy || pairing {
        let props = if a.legacy { props_of(&a.me, "1", false) } else { props_of(&a.me, "2", true) };
        out.push(Record { instance: format!("{} {}", a.me.name, a.me.id.short()), host: format!("linksy-{}.local.", a.me.id.short().to_lowercase()), props });
    }
    out
}

impl Discovery {
    pub(crate) fn start(announce: Announce, port: u16, tx: mpsc::UnboundedSender<DiscoveryEvent>) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let browse = daemon.browse(SERVICE_TYPE)?;
        let shared = Arc::new(Mutex::new(Shared { announce, registered: HashMap::new(), names: HashMap::new(), tags: HashMap::new() }));
        let wake = Arc::new(Notify::new());
        let this = Self { daemon, shared, wake, tx };
        this.announce(port);

        let (d, s) = (this.daemon.clone(), this.shared.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PRESENCE_CHECK_EVERY);
            tick.tick().await;
            loop {
                tick.tick().await;
                let visible: Vec<String> = s.lock().unwrap().names.keys().cloned().collect();
                for fullname in visible {
                    if d.verify(fullname, mdns_sd::VERIFY_TIMEOUT_DEFAULT).is_err() {
                        return;
                    }
                }
            }
        });

        let (s, tx) = (this.shared.clone(), this.tx.clone());
        tokio::spawn(async move {
            while let Ok(ev) = browse.recv_async().await {
                match ev {
                    ServiceEvent::ServiceResolved(r) => {
                        let mut addrs: Vec<SocketAddr> = r.addresses.iter().filter_map(|ip| socket_addr(ip, r.port)).collect();
                        // IPv4 first: link-local IPv6 needs a scope and fails more often.
                        addrs.sort_by_key(|a| (!a.is_ipv4(), a.to_string()));
                        if addrs.is_empty() {
                            continue;
                        }
                        let found = {
                            let mut s = s.lock().unwrap();
                            if let Some(t) = r.get_property_val_str("t") {
                                s.tags.insert(r.fullname.clone(), (t.to_string(), addrs.clone()));
                                recognise(&s.announce.peers, t)
                            } else {
                                parse_device(&r)
                            }
                            .inspect(|device| {
                                s.names.insert(r.fullname.clone(), device.id);
                            })
                        };
                        if let Some(device) = found
                            && tx.send(DiscoveryEvent::Found { device, addrs }).is_err()
                        {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let lost = {
                            let mut s = s.lock().unwrap();
                            s.tags.remove(&fullname);
                            forget_name(&mut s, &fullname)
                        };
                        if let Some(id) = lost {
                            let _ = tx.send(DiscoveryEvent::Lost { id });
                        }
                    }
                    other => debug!("mdns: {other:?}"),
                }
            }
        });

        let (d, s, wake) = (this.daemon.clone(), this.shared.clone(), this.wake.clone());
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(ANNOUNCE_CHECK_EVERY) => {}
                    _ = wake.notified() => {}
                }
                if d.status().is_err() {
                    break;
                }
                sync_records(&d, &s, port);
            }
        });
        Ok(this)
    }

    fn announce(&self, port: u16) {
        sync_records(&self.daemon, &self.shared, port);
    }

    /// New name, key, peers or pairing mode: announced right away, and records already
    /// seen are matched again against the paired devices' keys.
    pub(crate) fn update(&self, announce: Announce) {
        let mut events = Vec::new();
        {
            let mut s = self.shared.lock().unwrap();
            s.announce = announce;
            let tags: Vec<(String, String, Vec<SocketAddr>)> = s.tags.iter().map(|(n, (t, a))| (n.clone(), t.clone(), a.clone())).collect();
            for (fullname, t, addrs) in tags {
                match recognise(&s.announce.peers, &t) {
                    Some(device) if s.names.get(&fullname) != Some(&device.id) => {
                        s.names.insert(fullname, device.id);
                        events.push(DiscoveryEvent::Found { device, addrs });
                    }
                    Some(_) => {}
                    None => {
                        if let Some(id) = forget_name(&mut s, &fullname) {
                            events.push(DiscoveryEvent::Lost { id });
                        }
                    }
                }
            }
        }
        for ev in events {
            let _ = self.tx.send(ev);
        }
        self.wake.notify_one();
    }

    pub(crate) fn stop(self) {
        let _ = self.daemon.shutdown();
    }
}

/// Drop a record's name; the device is only gone when no other name maps to it (after a
/// rename or a new epoch, the old goodbye can arrive after the new announcement).
fn forget_name(s: &mut Shared, fullname: &str) -> Option<Fingerprint> {
    s.names.remove(fullname).filter(|id| !s.names.values().any(|v| v == id))
}

/// The paired device announcing tag `t`, allowing clocks a few minutes apart.
fn recognise(peers: &[(DeviceInfo, DiscoveryKey)], t: &str) -> Option<DeviceInfo> {
    let e = epoch_now();
    peers.iter().find(|(_, key)| [e.saturating_sub(1), e, e + 1].iter().any(|&e| tag(key, e) == t)).map(|(d, _)| d.clone())
}

/// Register the records wanted now, then say goodbye for the ones no longer wanted.
fn sync_records(daemon: &ServiceDaemon, shared: &Mutex<Shared>, port: u16) {
    let mut s = shared.lock().unwrap();
    let wanted = wanted(&s.announce, Instant::now());
    let mut keep = Vec::new();
    for Record { instance, host, props } in wanted {
        let info = match ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, &props[..]) {
            Ok(i) => i.enable_addr_auto(),
            Err(e) => {
                warn!("could not make the discovery record: {e}");
                continue;
            }
        };
        let fullname = info.get_fullname().to_string();
        if s.registered.get(&fullname) != Some(&props) {
            if s.registered.contains_key(&fullname) {
                let _ = daemon.unregister(&fullname);
            }
            match daemon.register(info) {
                Ok(()) => {
                    s.registered.insert(fullname.clone(), props);
                }
                Err(e) => warn!("could not announce on the network: {e}"),
            }
        }
        keep.push(fullname);
    }
    let stale: Vec<String> = s.registered.keys().filter(|n| !keep.contains(n)).cloned().collect();
    for fullname in stale {
        let _ = daemon.unregister(&fullname);
        s.registered.remove(&fullname);
    }
}

fn parse_device(s: &mdns_sd::ResolvedService) -> Option<DeviceInfo> {
    let id = s.get_property_val_str("id")?.parse().ok()?;
    let kind: DeviceKind = serde_json::from_value(serde_json::Value::String(s.get_property_val_str("kind")?.into())).ok()?;
    Some(DeviceInfo {
        id,
        name: s.get_property_val_str("name")?.to_string(),
        kind,
        os: s.get_property_val_str("os").unwrap_or("").to_string(),
        features: Vec::new(),
        away: None,
    })
}

fn socket_addr(ip: &ScopedIp, port: u16) -> Option<SocketAddr> {
    match ip {
        ScopedIp::V6(v6) if v6.addr().segments()[0] & 0xffc0 == 0xfe80 => {
            Some(SocketAddr::V6(SocketAddrV6::new(*v6.addr(), port, 0, v6.scope_id().index)))
        }
        other => match other.to_ip_addr() {
            IpAddr::V4(v4) if v4.is_loopback() => None,
            ip => Some(SocketAddr::new(ip, port)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test vector in docs/protocol.md §4.16, for other implementations.
    #[test]
    fn tag_matches_the_documented_vector() {
        let key: DiscoveryKey = std::array::from_fn(|i| i as u8);
        assert_eq!(tag(&key, 1_959_000), "9f88efcbdfd95b47eee92701da5f9d99");
        assert_ne!(tag(&key, 1_959_000), tag(&key, 1_959_001));
    }

    /// Checked against `openssl dgst -sha256 -mac HMAC`.
    #[test]
    fn hmac_is_hmac() {
        assert_eq!(hex::encode(hmac_sha256(&[0x0b; 32], b"Hi There")), "198a607eb44bfbc69903a0f1cf2bbdc5ba0aa3f3d9ae3c1c7a3b1696a0b68cf7");
    }
}
