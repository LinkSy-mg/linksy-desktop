//! Lock when you walk away: devices the user picked check in every few seconds while
//! they're around (`heartbeat`), and when one hasn't for the grace time, this computer
//! locks. Once per departure: it waits for the device to check in again before it can
//! lock again. It never unlocks anything.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use linksy_core::{DeviceInfo, Event, Fingerprint, Node};
use linksy_ipc::{LockPrefs, Shared};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

use crate::history::History;

#[derive(Clone)]
pub struct Away {
    path: PathBuf,
    inner: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    prefs: LockPrefs,
    /// When each device last checked in, and what it's called.
    seen: HashMap<Fingerprint, (Instant, DeviceInfo)>,
    /// Devices that checked in since this computer last locked for them.
    armed: HashSet<Fingerprint>,
}

/// How often a device checks in: three chances within the grace time, at most every 20 s.
fn every(grace_s: u32) -> u32 {
    (grace_s / 3).clamp(5, 20)
}

impl Away {
    pub fn load(path: PathBuf) -> Self {
        let prefs = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, inner: Arc::new(Mutex::new(State { prefs, ..Default::default() })) }
    }

    pub fn prefs(&self) -> LockPrefs {
        self.inner.lock().unwrap().prefs.clone()
    }

    /// Save new choices and tell the devices they concern.
    pub async fn set_prefs(&self, node: &Node, mut prefs: LockPrefs) -> Result<(), String> {
        prefs.grace_s = prefs.grace_s.clamp(15, 3600);
        let json = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())?;
        let before = std::mem::replace(&mut self.inner.lock().unwrap().prefs, prefs.clone());
        for id in &prefs.devices {
            // Ask; a device that's away hears it when it connects again.
            let _ = node.watch_heartbeat(id, true, every(prefs.grace_s)).await;
        }
        for id in before.devices.iter().filter(|id| !prefs.devices.contains(id)) {
            let _ = node.watch_heartbeat(id, false, 0).await;
            let mut state = self.inner.lock().unwrap();
            state.armed.remove(id);
            state.seen.remove(id);
        }
        Ok(())
    }

    pub async fn run(self, node: Node, history: History) {
        let mut events = node.subscribe();
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let gone = self.departed();
                    for device in gone {
                        info!("{} left: locking", device.name);
                        if lock_screen().await {
                            history.record_incoming(&device, Shared::Locked);
                            #[cfg(target_os = "linux")]
                            crate::notify::show(format!("Locked because {} left", device.name), "It locks whenever it leaves. Change this on its page in Linksy.".into());
                        }
                    }
                }
                event = events.recv() => match event {
                    Ok(Event::Heartbeat { from }) => {
                        let mut state = self.inner.lock().unwrap();
                        if state.prefs.devices.contains(&from.id) {
                            state.armed.insert(from.id);
                            state.seen.insert(from.id, (Instant::now(), from));
                        }
                    }
                    Ok(Event::Connected { device }) => {
                        let grace = {
                            let state = self.inner.lock().unwrap();
                            state.prefs.devices.contains(&device.id).then_some(state.prefs.grace_s)
                        };
                        if let Some(grace) = grace {
                            let _ = node.watch_heartbeat(&device.id, true, every(grace)).await;
                        }
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
            }
        }
    }

    /// Armed devices whose last check-in is older than the grace time; disarms them.
    fn departed(&self) -> Vec<DeviceInfo> {
        let mut state = self.inner.lock().unwrap();
        let grace = Duration::from_secs(state.prefs.grace_s.into());
        let gone: Vec<DeviceInfo> = state
            .armed
            .iter()
            .filter_map(|id| state.seen.get(id).filter(|(at, _)| at.elapsed() > grace).map(|(_, d)| d.clone()))
            .collect();
        for d in &gone {
            state.armed.remove(&d.id);
        }
        gone
    }
}

/// Lock the screen the way this desktop does it. Whether it's locked now.
async fn lock_screen() -> bool {
    // Tests and unusual setups name their own command.
    if let Ok(cmd) = std::env::var("LINKSY_LOCK_COMMAND") {
        return tokio::process::Command::new("sh").arg("-c").arg(cmd).status().await.is_ok_and(|s| s.success());
    }
    #[cfg(target_os = "linux")]
    if let Ok(bus) = zbus::Connection::session().await {
        for (name, path) in [("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"), ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver")] {
            if bus.call_method(Some(name), path, Some(name), "Lock", &()).await.is_ok() {
                return true;
            }
        }
    }
    match tokio::process::Command::new("loginctl").arg("lock-session").status().await {
        Ok(s) if s.success() => true,
        other => {
            warn!("could not lock the screen: {other:?}");
            false
        }
    }
}
