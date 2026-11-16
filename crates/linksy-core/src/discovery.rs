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
/// A browse queries more and more rarely, in the end once an hour, and a phone in power
/// saving easily misses the few tries to resolve a new record. So a fresh browse starts this
/// long after each new epoch, when paired devices announce under a new name...
const BROWSE_AFTER_EPOCH: Duration = Duration::from_secs(5);
/// ...and this long after a paired device's record went, in case its new one was missed.
const BROWSE_AFTER_LOST: Duration = Duration::from_secs(1);
/// Records kept at most. Anyone on the network can announce, so the memory (and the
/// presence checks) must not grow with what they send; the least recently seen records
/// go first, except private records a paired device's key makes (legacy records of paired
/// devices can go too: their plaintext id is forgeable). Bounds our maps only; mdns-sd
/// keeps its own cache.
const MAX_RECORDS: usize = 256;

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

/// How long until the next epoch starts, at `unix_secs`.
fn until_next_epoch(unix_secs: u64) -> Duration {
    Duration::from_secs(EPOCH_SECS - unix_secs % EPOCH_SECS)
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
    /// When each record was last seen: fullname → a counter that only goes up.
    seen: HashMap<String, u64>,
    clock: u64,
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
        let shared = Arc::new(Mutex::new(Shared { announce, registered: HashMap::new(), names: HashMap::new(), tags: HashMap::new(), seen: HashMap::new(), clock: 0 }));
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
                    match d.verify(fullname, mdns_sd::VERIFY_TIMEOUT_DEFAULT) {
                        Err(mdns_sd::Error::DaemonShutdown) => return,
                        // A full queue: the next round asks again.
                        Err(e) => debug!("mdns: could not check a device: {e}"),
                        Ok(()) => {}
                    }
                }
            }
        });

        let (d, s, tx) = (this.daemon.clone(), this.shared.clone(), this.tx.clone());
        tokio::spawn(async move {
            let mut browse = browse;
            loop {
                let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let again = tokio::time::sleep(until_next_epoch(unix) + BROWSE_AFTER_EPOCH);
                tokio::pin!(again);
                loop {
                    tokio::select! {
                        ev = browse.recv_async() => match ev.map(|ev| on_event(ev, &s, &tx)) {
                            Ok(Seen::Lost) => {
                                let soon = tokio::time::Instant::now() + BROWSE_AFTER_LOST;
                                if soon < again.deadline() {
                                    again.as_mut().reset(soon);
                                }
                            }
                            Ok(Seen::Other) => {}
                            Ok(Seen::Closed) | Err(_) => return,
                        },
                        _ = &mut again => break,
                    }
                }
                // Browsing again replaces the listener; the old one's queries stop with it.
                match d.browse(SERVICE_TYPE) {
                    Ok(b) => browse = b,
                    Err(mdns_sd::Error::DaemonShutdown) => return,
                    Err(e) => debug!("mdns: could not browse again: {e}"),
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

/// What a browse event came to.
enum Seen {
    /// A paired or pairing device is gone.
    Lost,
    Other,
    /// The node stopped listening.
    Closed,
}

fn on_event(ev: ServiceEvent, s: &Mutex<Shared>, tx: &mpsc::UnboundedSender<DiscoveryEvent>) -> Seen {
    match ev {
        ServiceEvent::ServiceResolved(r) => {
            let mut addrs: Vec<SocketAddr> = r.addresses.iter().filter_map(|ip| socket_addr(ip, r.port)).collect();
            // IPv4 first: link-local IPv6 needs a scope and fails more often.
            addrs.sort_by_key(|a| (!a.is_ipv4(), a.to_string()));
            if addrs.is_empty() {
                return Seen::Other;
            }
            let legacy = if r.get_property_val_str("t").is_none() { parse_device(&r) } else { None };
            let tag = r.get_property_val_str("t").map(str::to_string);
            let (found, evicted) = ingest_resolved(&mut s.lock().unwrap(), &r.fullname, tag, legacy, addrs.clone(), MAX_RECORDS);
            for id in evicted {
                if tx.send(DiscoveryEvent::Lost { id }).is_err() {
                    return Seen::Closed;
                }
            }
            if let Some(device) = found
                && tx.send(DiscoveryEvent::Found { device, addrs }).is_err()
            {
                return Seen::Closed;
            }
            Seen::Other
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            let lost = ingest_removed(&mut s.lock().unwrap(), &fullname);
            match lost {
                Some(id) => match tx.send(DiscoveryEvent::Lost { id }) {
                    Ok(()) => Seen::Lost,
                    Err(_) => Seen::Closed,
                },
                None => Seen::Other,
            }
        }
        other => {
            debug!("mdns: {other:?}");
            Seen::Other
        }
    }
}

/// A resolved record, under the lock: remember it, then evict over `cap`. `tag` is the
/// private record's tag, `legacy` the device a §4.1 record names. Returns the device it
/// stands for (if recognised) and the devices that went with the evicted records.
fn ingest_resolved(s: &mut Shared, fullname: &str, tag: Option<String>, legacy: Option<DeviceInfo>, addrs: Vec<SocketAddr>, cap: usize) -> (Option<DeviceInfo>, Vec<Fingerprint>) {
    let found = match tag {
        Some(t) => {
            let found = recognise(&s.announce.peers, &t);
            s.tags.insert(fullname.to_string(), (t, addrs));
            found
        }
        None => legacy,
    };
    if let Some(device) = &found {
        s.names.insert(fullname.to_string(), device.id);
    }
    if s.tags.contains_key(fullname) || s.names.contains_key(fullname) {
        s.note_seen(fullname);
    }
    (found, evict_over_cap(s, cap))
}

/// A record's goodbye, under the lock: forget it everywhere. Returns the device if that
/// was the last name it had.
fn ingest_removed(s: &mut Shared, fullname: &str) -> Option<Fingerprint> {
    s.tags.remove(fullname);
    s.seen.remove(fullname);
    forget_name(s, fullname)
}

impl Shared {
    fn note_seen(&mut self, fullname: &str) {
        // Saturating: if it ever got there, ties just fall back to name order.
        self.clock = self.clock.saturating_add(1);
        self.seen.insert(fullname.to_string(), self.clock);
    }
}

/// Forget the least recently seen records until at most `cap` remain. A private record
/// whose tag a paired device's key makes is never dropped: nobody else can forge one. A
/// legacy §4.1 record (no tag, keyed by a plaintext id anyone can copy) is not protected,
/// even a paired device's, and can be evicted; it comes back when it is announced again.
/// The bound is for our own maps: mdns-sd keeps its own cache, so an evicted record may
/// show up again on the next browse. Returns the devices that went with the records.
fn evict_over_cap(s: &mut Shared, cap: usize) -> Vec<Fingerprint> {
    let mut all: Vec<String> = s.tags.keys().chain(s.names.keys()).cloned().collect();
    all.sort();
    all.dedup();
    if all.len() <= cap {
        return Vec::new();
    }
    let mut droppable: Vec<(u64, String)> =
        all.iter().filter(|n| !s.tags.get(*n).is_some_and(|(t, _)| recognise(&s.announce.peers, t).is_some())).map(|n| (s.seen.get(n).copied().unwrap_or(0), n.clone())).collect();
    droppable.sort();
    let excess = all.len() - cap;
    let mut lost = Vec::new();
    for (_, fullname) in droppable.into_iter().take(excess) {
        s.tags.remove(&fullname);
        s.seen.remove(&fullname);
        lost.extend(forget_name(s, &fullname));
    }
    lost
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
    Some(crate::settings::tidy_peer(DeviceInfo {
        id,
        name: s.get_property_val_str("name")?.to_string(),
        kind,
        os: s.get_property_val_str("os").unwrap_or("").to_string(),
        features: Vec::new(),
        away: None,
    }))
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

    #[test]
    fn next_epoch_is_never_now() {
        assert_eq!(until_next_epoch(1_959_000 * EPOCH_SECS), Duration::from_secs(EPOCH_SECS));
        assert_eq!(until_next_epoch(1_959_000 * EPOCH_SECS + 899), Duration::from_secs(1));
        assert_eq!(until_next_epoch(1_959_000 * EPOCH_SECS + 300), Duration::from_secs(600));
    }

    #[test]
    fn announced_names_are_cleaned() {
        let device = |name: &str| DeviceInfo { id: crate::identity::Fingerprint::of(b"x"), name: name.into(), kind: DeviceKind::Phone, os: String::new(), features: Vec::new(), away: None };
        assert_eq!(crate::settings::tidy_peer(device("Pixel 9")).name, "Pixel 9");
        assert_eq!(crate::settings::tidy_peer(device("A\nB\u{202e}")).name, "A B");
        let long = crate::settings::tidy_peer(device(&"x".repeat(5000))).name;
        assert_eq!(long.chars().count(), crate::MAX_NAME);
        let fallback = crate::settings::tidy_peer(device("\u{202e}\n"));
        assert_eq!(fallback.name, fallback.id.short());
    }

    fn shared_with(peers: Vec<(DeviceInfo, DiscoveryKey)>) -> Shared {
        let me = DeviceInfo { id: Fingerprint::of(b"me"), name: "me".into(), kind: DeviceKind::Desktop, os: String::new(), features: Vec::new(), away: None };
        let announce = Announce { me, key: [0; 32], legacy: false, pairing_until: None, peers };
        Shared { announce, registered: HashMap::new(), names: HashMap::new(), tags: HashMap::new(), seen: HashMap::new(), clock: 0 }
    }

    fn see_tag(s: &mut Shared, name: &str, t: &str) {
        s.tags.insert(name.into(), (t.into(), Vec::new()));
        s.note_seen(name);
    }

    #[test]
    fn records_are_capped_oldest_first() {
        let mut s = shared_with(Vec::new());
        for i in 0..10 {
            see_tag(&mut s, &format!("r{i}"), "junk");
        }
        // Seen again: no longer the oldest.
        s.note_seen("r0");
        assert!(evict_over_cap(&mut s, 10).is_empty());
        see_tag(&mut s, "r10", "junk");
        evict_over_cap(&mut s, 10);
        assert_eq!(s.tags.len(), 10);
        assert!(s.tags.contains_key("r0") && s.tags.contains_key("r10"));
        assert!(!s.tags.contains_key("r1"));
        assert_eq!(s.seen.len(), 10);
    }

    #[test]
    fn a_flood_never_evicts_a_paired_devices_record() {
        let key: DiscoveryKey = [7; 32];
        let device = DeviceInfo { id: Fingerprint::of(b"phone"), name: "p".into(), kind: DeviceKind::Phone, os: String::new(), features: Vec::new(), away: None };
        let mut s = shared_with(vec![(device.clone(), key)]);
        let t = tag(&key, epoch_now());
        see_tag(&mut s, "paired", &t);
        s.names.insert("paired".into(), device.id);
        for i in 0..50 {
            see_tag(&mut s, &format!("junk{i}"), "junk");
            let lost = evict_over_cap(&mut s, 5);
            assert!(lost.is_empty());
        }
        assert_eq!(s.tags.len(), 5);
        assert!(s.tags.contains_key("paired") && s.names.contains_key("paired"));
    }

    #[test]
    fn evicting_a_named_device_reports_it_lost() {
        let mut s = shared_with(Vec::new());
        let id = Fingerprint::of(b"stranger");
        s.names.insert("old".into(), id);
        s.note_seen("old");
        see_tag(&mut s, "new", "junk");
        assert_eq!(evict_over_cap(&mut s, 1), vec![id]);
        assert!(s.names.is_empty());
    }

    fn dev(seed: &[u8]) -> DeviceInfo {
        DeviceInfo { id: Fingerprint::of(seed), name: "d".into(), kind: DeviceKind::Phone, os: String::new(), features: Vec::new(), away: None }
    }

    #[test]
    fn ingest_enforces_the_cap_and_reports_evicted_devices() {
        let mut s = shared_with(Vec::new());
        let stranger = dev(b"stranger");
        ingest_resolved(&mut s, "legacy0", None, Some(stranger.clone()), Vec::new(), 3);
        for i in 0..2 {
            ingest_resolved(&mut s, &format!("t{i}"), Some("junk".into()), None, Vec::new(), 3);
        }
        assert_eq!(s.tags.len() + s.names.len(), 3);
        let (found, lost) = ingest_resolved(&mut s, "t2", Some("junk".into()), None, Vec::new(), 3);
        assert!(found.is_none());
        assert_eq!(lost, vec![stranger.id]);
        assert_eq!((s.tags.len(), s.names.len(), s.seen.len()), (3, 0, 3));
    }

    #[test]
    fn ingest_keeps_a_paired_private_record_but_may_evict_a_legacy_one() {
        let key: DiscoveryKey = [7; 32];
        let phone = dev(b"phone");
        let mut s = shared_with(vec![(phone.clone(), key)]);
        let (found, _) = ingest_resolved(&mut s, "paired", Some(tag(&key, epoch_now())), None, Vec::new(), 2);
        assert_eq!(found.map(|d| d.id), Some(phone.id));
        ingest_resolved(&mut s, "legacy", None, Some(phone.clone()), Vec::new(), 2);
        let (_, lost) = ingest_resolved(&mut s, "junk", Some("junk".into()), None, Vec::new(), 2);
        // The oldest droppable one is the legacy record; the device still has its private name.
        assert!(lost.is_empty());
        assert!(s.tags.contains_key("paired") && s.tags.contains_key("junk"));
        assert!(s.names.contains_key("paired") && !s.names.contains_key("legacy"));
    }

    #[test]
    fn a_goodbye_forgets_the_record_everywhere() {
        let mut s = shared_with(Vec::new());
        let d = dev(b"x");
        ingest_resolved(&mut s, "a", None, Some(d.clone()), Vec::new(), 10);
        ingest_resolved(&mut s, "b", Some("junk".into()), None, Vec::new(), 10);
        assert_eq!(ingest_removed(&mut s, "a"), Some(d.id));
        assert_eq!(ingest_removed(&mut s, "b"), None);
        assert!(s.tags.is_empty() && s.names.is_empty() && s.seen.is_empty());
    }

    #[test]
    fn the_seen_counter_saturates() {
        let mut s = shared_with(Vec::new());
        s.clock = u64::MAX;
        s.note_seen("a");
        assert_eq!(s.clock, u64::MAX);
    }

    /// Checked against `openssl dgst -sha256 -mac HMAC`.
    #[test]
    fn hmac_is_hmac() {
        assert_eq!(hex::encode(hmac_sha256(&[0x0b; 32], b"Hi There")), "198a607eb44bfbc69903a0f1cf2bbdc5ba0aa3f3d9ae3c1c7a3b1696a0b68cf7");
    }
}
