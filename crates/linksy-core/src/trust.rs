//! Paired devices, persisted as JSON next to the identity.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::identity::{DeviceInfo, Fingerprint};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub device: DeviceInfo,
    /// Where we last reached it. Dialed directly at startup, before discovery answers.
    pub last_addr: Option<SocketAddr>,
    pub paired_at: u64,
    /// When it was last connected on the same network or by cable (seconds since the
    /// epoch): away from home works for [`crate::away::LEASE`] after that.
    #[serde(default)]
    pub last_met: Option<u64>,
    /// Its user allowed everything from away for it: screens, control, files, camera,
    /// microphone. Off: only the small things (clipboard, links, ring, battery, files sent).
    #[serde(default)]
    pub away_all: bool,
    /// The key its private discovery tags are made from (docs/protocol.md §4.16), hex.
    /// `None`: it doesn't speak §4.16 yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_key: Option<String>,
    /// Files from it are accepted without asking.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_accept: bool,
    /// It sees and controls the screen without asking ([`SessionKind::may_always_allow`]),
    /// on the same network or cable; from away it's always asked.
    ///
    /// [`SessionKind::may_always_allow`]: crate::protocol::SessionKind::may_always_allow
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_screen: bool,
}

impl TrustedDevice {
    /// Whether it may connect from away now: met on the same network or by cable within
    /// the lease (pairing counts as meeting).
    pub fn lease_ok(&self) -> bool {
        let met = self.last_met.unwrap_or(self.paired_at);
        now().saturating_sub(met) < crate::away::LEASE.as_secs()
    }

    /// When away access ends, in seconds since the epoch.
    pub fn lease_until(&self) -> u64 {
        self.last_met.unwrap_or(self.paired_at) + crate::away::LEASE.as_secs()
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub struct TrustStore {
    path: Option<PathBuf>,
    devices: BTreeMap<Fingerprint, TrustedDevice>,
}

impl TrustStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        let devices = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Vec<TrustedDevice>>(&bytes)?
                .into_iter()
                .map(|d| (d.device.id, d))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path: Some(path), devices })
    }

    pub fn in_memory() -> Self {
        Self { path: None, devices: BTreeMap::new() }
    }

    pub fn get(&self, id: &Fingerprint) -> Option<&TrustedDevice> {
        self.devices.get(id)
    }

    pub fn contains(&self, id: &Fingerprint) -> bool {
        self.devices.contains_key(id)
    }

    pub fn list(&self) -> Vec<TrustedDevice> {
        self.devices.values().cloned().collect()
    }

    pub fn add(&mut self, device: DeviceInfo, addr: Option<SocketAddr>) -> Result<()> {
        let paired_at = now();
        self.devices.insert(device.id, TrustedDevice { device, last_addr: addr, paired_at, last_met: Some(paired_at), away_all: false, discovery_key: None, auto_accept: false, allow_screen: false });
        self.save()
    }

    /// Refresh name and address after a successful reconnect.
    pub fn touch(&mut self, device: &DeviceInfo, addr: SocketAddr) -> Result<()> {
        let Some(d) = self.devices.get_mut(&device.id) else { return Ok(()) };
        // Meeting on the same network or by cable renews away access (saved at most hourly).
        let met = crate::node::link_of(addr) != crate::node::Link::Away;
        let renew = met && d.last_met.is_none_or(|m| now().saturating_sub(m) >= 3600);
        if d.device == *device && d.last_addr == Some(addr) && !renew {
            return Ok(());
        }
        if renew {
            d.last_met = Some(now());
        }
        // Away from home it has no new address to give; keep the one it had.
        let mut device = device.clone();
        if device.away.is_none() {
            device.away = d.device.away.take();
        }
        d.device = device;
        // A USB or away bridge's address is only good while it lasts: keep the network
        // one (a cable's, for lack of one: tests on one machine have nothing else).
        match crate::node::link_of(addr) {
            crate::node::Link::Lan => d.last_addr = Some(addr),
            crate::node::Link::Usb if d.last_addr.is_none() => d.last_addr = Some(addr),
            _ => {}
        }
        self.save()
    }

    /// Allow everything from away for `id`, or only the small things.
    pub fn set_away_all(&mut self, id: &Fingerprint, on: bool) -> Result<bool> {
        let Some(d) = self.devices.get_mut(id) else { return Ok(false) };
        d.away_all = on;
        self.save()?;
        Ok(true)
    }

    /// Accept files from `id` without asking, or ask again.
    pub fn set_auto_accept(&mut self, id: &Fingerprint, on: bool) -> Result<bool> {
        let Some(d) = self.devices.get_mut(id) else { return Ok(false) };
        d.auto_accept = on;
        self.save()?;
        Ok(true)
    }

    /// Let `id` see and control the screen without asking, or ask again.
    pub fn set_allow_screen(&mut self, id: &Fingerprint, on: bool) -> Result<bool> {
        let Some(d) = self.devices.get_mut(id) else { return Ok(false) };
        d.allow_screen = on;
        self.save()?;
        Ok(true)
    }

    /// Store `id`'s discovery key. Returns whether it changed.
    pub fn set_discovery_key(&mut self, id: &Fingerprint, key: &str) -> Result<bool> {
        let Some(d) = self.devices.get_mut(id) else { return Ok(false) };
        if d.discovery_key.as_deref() == Some(key) {
            return Ok(false);
        }
        d.discovery_key = Some(key.to_string());
        self.save()?;
        Ok(true)
    }

    pub fn remove(&mut self, id: &Fingerprint) -> Result<bool> {
        let removed = self.devices.remove(id).is_some();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let list: Vec<_> = self.devices.values().collect();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&list)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
