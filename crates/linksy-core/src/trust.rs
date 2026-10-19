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
        let paired_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.devices.insert(device.id, TrustedDevice { device, last_addr: addr, paired_at });
        self.save()
    }

    /// Refresh name and address after a successful reconnect.
    pub fn touch(&mut self, device: &DeviceInfo, addr: SocketAddr) -> Result<()> {
        let Some(d) = self.devices.get_mut(&device.id) else { return Ok(()) };
        if d.device == *device && d.last_addr == Some(addr) {
            return Ok(());
        }
        d.device = device.clone();
        d.last_addr = Some(addr);
        self.save()
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
