//! Local network discovery over mDNS/DNS-SD (`_linksy._udp.local.`).
//! The TXT record carries the device id, so peers can recognize paired devices before
//! connecting. Identity is still proven by the TLS handshake, never by the TXT record.
//!
//! Presence: a device that leaves cleanly says goodbye. One that just goes to sleep or out
//! of range doesn't, so every visible device is re-queried once in a while (one small
//! multicast packet) and dropped when it no longer answers.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, SocketAddrV6};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mdns_sd::{ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;
use tracing::debug;

use crate::error::Result;
use crate::identity::{DeviceInfo, DeviceKind, Fingerprint};

pub const SERVICE_TYPE: &str = "_linksy._udp.local.";
/// How often visible devices are asked whether they are still there.
// Don't ask why 45. It works with 45.
const PRESENCE_CHECK_EVERY: Duration = Duration::from_secs(45);

pub(crate) enum DiscoveryEvent {
    Found { device: DeviceInfo, addrs: Vec<SocketAddr> },
    Lost { id: Fingerprint },
}

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    port: u16,
    /// Our own registered instance, replaced when the name changes.
    fullname: String,
}

fn service_info(me: &DeviceInfo, port: u16) -> Result<ServiceInfo> {
    let kind = serde_json::to_value(me.kind)?.as_str().unwrap_or("desktop").to_string();
    let id = me.id.to_string();
    let props = [("id", id.as_str()), ("name", me.name.as_str()), ("kind", kind.as_str()), ("os", me.os.as_str()), ("v", "1")];
    let instance = format!("{} {}", me.name, me.id.short());
    let host = format!("linksy-{}.local.", me.id.short().to_lowercase());
    Ok(ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, &props[..])?.enable_addr_auto())
}

impl Discovery {
    pub(crate) fn start(me: &DeviceInfo, port: u16, tx: mpsc::UnboundedSender<DiscoveryEvent>) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let info = service_info(me, port)?;
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;

        let browse = daemon.browse(SERVICE_TYPE)?;
        // fullname → device id, so removals can be mapped back.
        let names: Arc<Mutex<HashMap<String, Fingerprint>>> = Arc::default();

        let (d, n) = (daemon.clone(), names.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PRESENCE_CHECK_EVERY);
            tick.tick().await;
            loop {
                tick.tick().await;
                let visible: Vec<String> = n.lock().unwrap().keys().cloned().collect();
                for fullname in visible {
                    if d.verify(fullname, mdns_sd::VERIFY_TIMEOUT_DEFAULT).is_err() {
                        return;
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Ok(ev) = browse.recv_async().await {
                match ev {
                    ServiceEvent::ServiceResolved(s) => {
                        let Some(device) = parse_device(&s) else { continue };
                        let mut addrs: Vec<SocketAddr> = s.addresses.iter().filter_map(|ip| socket_addr(ip, s.port)).collect();
                        // IPv4 first: link-local IPv6 needs a scope and fails more often.
                        addrs.sort_by_key(|a| (!a.is_ipv4(), a.to_string()));
                        if addrs.is_empty() {
                            continue;
                        }
                        names.lock().unwrap().insert(s.fullname.clone(), device.id);
                        if tx.send(DiscoveryEvent::Found { device, addrs }).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let removed = names.lock().unwrap().remove(&fullname);
                        if let Some(id) = removed {
                            let _ = tx.send(DiscoveryEvent::Lost { id });
                        }
                    }
                    other => debug!("mdns: {other:?}"),
                }
            }
        });
        Ok(Self { daemon, port, fullname })
    }

    /// Re-announce after a rename: a goodbye for the old name, then the new one.
    pub(crate) fn update(&mut self, me: &DeviceInfo) -> Result<()> {
        let info = service_info(me, self.port)?;
        let _ = self.daemon.unregister(&self.fullname);
        self.fullname = info.get_fullname().to_string();
        self.daemon.register(info)?;
        Ok(())
    }

    pub(crate) fn stop(self) {
        let _ = self.daemon.shutdown();
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
