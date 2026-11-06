//! A phone's controls (volumes, ringer, Do Not Disturb, flashlight; docs/protocol.md §4.24).
//! The service asks each phone that has them to say how they are, keeps the latest for the
//! apps, and applies what they set. Where a phone is set to, it also keeps this computer's Do
//! Not Disturb and the phone's the same ("Keep Do Not Disturb in Sync"): the last change wins.
//!
//! A phone only speaks when one of them changes, so watching every connected phone costs it
//! nothing while they are left alone.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use linksy_core::{ControlsChange, ControlsState, Dnd, Event, Fingerprint, Node};
use linksy_ipc::PhoneControlsInfo;
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

pub static CONTROLS: OnceLock<Controls> = OnceLock::new();

/// How often this computer's Do Not Disturb is looked at while a phone follows it.
const LOOK: Duration = Duration::from_secs(2);

pub struct Controls {
    path: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Each connected phone's controls, as it last said.
    states: HashMap<Fingerprint, ControlsState>,
    /// The phones whose Do Not Disturb follows this computer's, and the other way round.
    sync: HashSet<Fingerprint>,
}

impl Controls {
    pub fn load(path: PathBuf) -> Self {
        let sync: HashSet<Fingerprint> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, inner: Mutex::new(Inner { sync, ..Inner::default() }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What the apps show for a phone.
    pub async fn info(&self, phone: &Fingerprint) -> PhoneControlsInfo {
        let can_sync = crate::controls::shared().probe().await.dnd();
        let inner = self.lock();
        PhoneControlsInfo { state: inner.states.get(phone).copied(), sync_dnd: inner.sync.contains(phone), can_sync }
    }

    pub fn set_sync(&self, phone: Fingerprint, on: bool) -> Result<(), String> {
        let mut inner = self.lock();
        let changed = if on { inner.sync.insert(phone) } else { inner.sync.remove(&phone) };
        if changed {
            let bytes = serde_json::to_vec(&inner.sync).map_err(|e| e.to_string())?;
            std::fs::write(&self.path, bytes).map_err(|e| format!("could not save: {e}"))?;
        }
        Ok(())
    }

    fn keeps(&self, phone: &Fingerprint) -> bool {
        self.lock().sync.contains(phone)
    }

    fn remember(&self, phone: Fingerprint, state: ControlsState) {
        self.lock().states.insert(phone, state);
    }

    /// The phones that follow this computer and can be told to: they gave Linksy Do Not
    /// Disturb access.
    fn followers(&self) -> Vec<(Fingerprint, Dnd)> {
        let inner = self.lock();
        inner.sync.iter().filter_map(|p| inner.states.get(p).filter(|s| s.dnd_access).map(|s| (*p, s.dnd))).collect()
    }
}

/// On means anything the phone lets through is held back, however it does it.
fn is_on(dnd: Dnd) -> bool {
    !matches!(dnd, Dnd::Off | Dnd::Unknown)
}

pub async fn run(node: Node) {
    let Some(controls) = CONTROLS.get() else { return };
    // What this computer has for Do Not Disturb, found once
    crate::controls::shared().probe().await;
    let mut events = node.subscribe();
    // What each side last was, so only a change on one is carried to the other
    let mut seen: HashMap<Fingerprint, bool> = HashMap::new();
    let mut computer: Option<bool> = None;
    let mut every = tokio::time::interval(LOOK);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Event::Connected { device }) if device.features.iter().any(|f| f == "phone_controls") => {
                    let node = node.clone();
                    tokio::spawn(async move {
                        if let Err(e) = node.watch_controls(&device.id, true).await {
                            debug!("controls: could not ask {}: {e}", device.name);
                        }
                    });
                }
                // The last it said stays for its page: a link closes after a minute of nothing, and a
                // phone away from home is on none. Opening the page asks again (it dials).
                Ok(Event::Disconnected { id }) => {
                    seen.remove(&id);
                }
                Ok(Event::ControlsState { from, state }) => {
                    controls.remember(from.id, state);
                    let now = is_on(state.dnd);
                    let before = seen.insert(from.id, now);
                    // The phone's own change, with a computer that has a Do Not Disturb to follow
                    if controls.keeps(&from.id) && before.is_some_and(|b| b != now) && computer != Some(now) && crate::controls::shared().set_dnd(now).await {
                        computer = Some(now);
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
            _ = every.tick() => {
                let followers = controls.followers();
                if followers.is_empty() {
                    computer = None;
                    continue;
                }
                let Some(now) = crate::controls::shared().dnd().await else { continue };
                let before = computer.replace(now);
                if before.is_some_and(|b| b != now) {
                    for (phone, dnd) in followers {
                        if is_on(dnd) == now {
                            continue;
                        }
                        // The computer's switch is only on or off: on is what the phone's own
                        // settings let through
                        let change = ControlsChange { dnd: Some(if now { Dnd::Priority } else { Dnd::Off }), ..ControlsChange::default() };
                        seen.insert(phone, now);
                        if let Err(e) = node.set_controls(&phone, change).await {
                            debug!("controls: Do Not Disturb not sent: {e}");
                        }
                    }
                }
            }
        }
    }
}
