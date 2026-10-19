//! Activity history: what was shared with which device, kept even while no window is open.
//!
//! Stored as JSON in the data dir, newest last, capped at [`KEEP`] entries. Clipboard text
//! stays in memory only; on disk a clipboard entry just says that text was shared.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use linksy_core::{DeviceInfo, Direction, Event, FileEntry, Fingerprint, Node, SessionKind, Uuid};
use linksy_ipc::{HistoryEntry, HistoryStatus, Shared};
use tokio::sync::broadcast::{self, error::RecvError};
use tracing::warn;

const KEEP: usize = 500;

#[derive(Clone)]
pub struct History {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    entries: Mutex<Vec<HistoryEntry>>,
    updates: broadcast::Sender<HistoryEntry>,
}

impl History {
    pub fn load(path: PathBuf) -> Self {
        let mut entries: Vec<HistoryEntry> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // Whatever was running when the service stopped didn't finish.
        for e in &mut entries {
            if e.status == HistoryStatus::InProgress {
                e.status = HistoryStatus::Failed { reason: "interrupted".into() };
            }
        }
        let (updates, _) = broadcast::channel(128);
        Self { inner: Arc::new(Inner { path, entries: Mutex::new(entries), updates }) }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HistoryEntry> {
        self.inner.updates.subscribe()
    }

    /// Newest first, optionally only with one device.
    pub fn list(&self, with: Option<Fingerprint>) -> Vec<HistoryEntry> {
        self.inner.entries.lock().unwrap().iter().rev().filter(|e| with.is_none_or(|id| e.peer == id)).cloned().collect()
    }

    pub fn clear(&self, with: Option<Fingerprint>) {
        self.inner.entries.lock().unwrap().retain(|e| with.is_some_and(|id| e.peer != id));
        self.save();
    }

    /// Record something sent from here that has no core event (clipboard, ping).
    pub fn record_sent(&self, to: &DeviceInfo, what: Shared) {
        self.upsert(HistoryEntry {
            id: Uuid::new_v4(),
            at: now_ms(),
            peer: to.id,
            peer_name: to.name.clone(),
            direction: Direction::Outgoing,
            what,
            status: HistoryStatus::Done,
        });
    }

    fn upsert(&self, entry: HistoryEntry) {
        {
            let mut entries = self.inner.entries.lock().unwrap();
            match entries.iter_mut().find(|e| e.id == entry.id) {
                Some(e) => *e = entry.clone(),
                None => {
                    entries.push(entry.clone());
                    let excess = entries.len().saturating_sub(KEEP);
                    entries.drain(..excess);
                }
            }
        }
        if entry.status != HistoryStatus::InProgress {
            self.save();
        }
        let _ = self.inner.updates.send(entry);
    }

    fn update(&self, id: Uuid, f: impl FnOnce(&mut HistoryEntry)) {
        let entry = {
            let mut entries = self.inner.entries.lock().unwrap();
            let Some(e) = entries.iter_mut().find(|e| e.id == id) else { return };
            f(e);
            e.clone()
        };
        self.upsert(entry);
    }

    fn save(&self) {
        let mut entries = self.inner.entries.lock().unwrap().clone();
        for e in &mut entries {
            if let Shared::Clipboard { text } = &mut e.what {
                text.clear();
            }
        }
        let tmp = self.inner.path.with_extension("json.tmp");
        let written = serde_json::to_vec(&entries).map_err(std::io::Error::other).and_then(|b| std::fs::write(&tmp, b)).and_then(|_| std::fs::rename(&tmp, &self.inner.path));
        if let Err(e) = written {
            warn!("could not save history: {e}");
        }
    }

    /// Follow the node's events and turn them into history.
    pub async fn run(self, node: Node) {
        let mut events = node.subscribe();
        // Offers waiting for an answer, so a decline can still be recorded.
        let mut offers: HashMap<Uuid, (DeviceInfo, Vec<FileEntry>, u64)> = HashMap::new();
        let mut started: HashMap<Uuid, std::time::Instant> = HashMap::new();
        loop {
            let event = match events.recv().await {
                Ok(e) => e,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            match event {
                Event::TransferRequest { transfer, from, files, total } => {
                    offers.insert(transfer, (from, files, total));
                }
                Event::TransferStarted { transfer, peer, direction, files, total } => {
                    offers.remove(&transfer);
                    self.upsert(HistoryEntry {
                        id: transfer,
                        at: now_ms(),
                        peer: peer.id,
                        peer_name: peer.name,
                        direction,
                        what: Shared::Files { files: files.into_iter().map(|f| f.path).collect(), total, saved: Vec::new() },
                        status: HistoryStatus::InProgress,
                    });
                }
                Event::TransferComplete { transfer, saved } => self.update(transfer, |e| {
                    e.status = HistoryStatus::Done;
                    if let Shared::Files { saved: s, .. } = &mut e.what {
                        *s = saved;
                    }
                }),
                Event::TransferFailed { transfer, reason } => {
                    let status = match reason.as_str() {
                        "declined" | "request was declined" => HistoryStatus::Declined,
                        "cancelled" => HistoryStatus::Cancelled,
                        _ => HistoryStatus::Failed { reason },
                    };
                    match offers.remove(&transfer) {
                        // Declined before it started: there's no entry yet.
                        Some((from, files, total)) => self.upsert(HistoryEntry {
                            id: transfer,
                            at: now_ms(),
                            peer: from.id,
                            peer_name: from.name,
                            direction: Direction::Incoming,
                            what: Shared::Files { files: files.into_iter().map(|f| f.path).collect(), total, saved: Vec::new() },
                            status,
                        }),
                        None => self.update(transfer, |e| e.status = status),
                    }
                }
                Event::Clipboard { from, text } => self.upsert(incoming(&from, Shared::Clipboard { text })),
                Event::Notification { from, title, body } => self.upsert(incoming(&from, Shared::Ping { title, body })),
                Event::SessionStarted { session, peer, sharing, kind } => {
                    started.insert(session, std::time::Instant::now());
                    self.upsert(HistoryEntry {
                        id: session,
                        at: now_ms(),
                        peer: peer.id,
                        peer_name: peer.name,
                        // Incoming: their screen came to us.
                        direction: if sharing { Direction::Outgoing } else { Direction::Incoming },
                        what: match kind {
                            SessionKind::Camera => Shared::Camera { seconds: 0 },
                            _ => Shared::Screen { seconds: 0, display: kind == SessionKind::Display },
                        },
                        status: HistoryStatus::InProgress,
                    });
                }
                Event::SessionEnded { session } => {
                    let seconds = started.remove(&session).map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    self.update(session, |e| {
                        e.status = HistoryStatus::Done;
                        if let Shared::Screen { seconds: s, .. } | Shared::Camera { seconds: s } = &mut e.what {
                            *s = seconds;
                        }
                    });
                }
                Event::DeviceUpdated { device } => {
                    // Keep names current in past entries too.
                    let mut entries = self.inner.entries.lock().unwrap();
                    for e in entries.iter_mut().filter(|e| e.peer == device.id) {
                        e.peer_name = device.name.clone();
                    }
                }
                _ => {}
            }
        }
    }
}

fn incoming(from: &DeviceInfo, what: Shared) -> HistoryEntry {
    HistoryEntry {
        id: Uuid::new_v4(),
        at: now_ms(),
        peer: from.id,
        peer_name: from.name.clone(),
        direction: Direction::Incoming,
        what,
        status: HistoryStatus::Done,
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(peer: Fingerprint, what: Shared) -> HistoryEntry {
        HistoryEntry { id: Uuid::new_v4(), at: 1, peer, peer_name: "Peer".into(), direction: Direction::Incoming, what, status: HistoryStatus::Done }
    }

    #[test]
    fn keeps_clipboard_text_off_disk_and_caps_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let h = History::load(path.clone());
        let a = Fingerprint([1; 32]);
        let b = Fingerprint([2; 32]);
        h.upsert(entry(a, Shared::Clipboard { text: "hunter2".into() }));
        h.upsert(entry(b, Shared::Ping { title: "Ping".into(), body: String::new() }));
        assert_eq!(h.list(None).len(), 2);
        assert_eq!(h.list(Some(a)).len(), 1);
        // In memory the text is there; on disk it isn't.
        assert!(matches!(&h.list(Some(a))[0].what, Shared::Clipboard { text } if text == "hunter2"));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("hunter2"));

        let reloaded = History::load(path);
        assert_eq!(reloaded.list(None).len(), 2);
        reloaded.clear(Some(a));
        assert_eq!(reloaded.list(None).len(), 1);
        for _ in 0..KEEP + 10 {
            reloaded.upsert(entry(b, Shared::Ping { title: "x".into(), body: String::new() }));
        }
        assert_eq!(reloaded.list(None).len(), KEEP);
    }
}
