//! This computer's notifications on phones (docs/protocol.md §4.25). A phone that asks for them
//! (a switch on its page, off until turned on) gets each notification that comes up here, as
//! `computer_notification`: app, title and text. The notifications are listened for on the
//! session bus as a monitor, which reads what applications send to the notification server
//! without taking its place, so nothing about how they show here changes.
//!
//! Not forwarded: Linksy's own (a phone's notifications shown here must not go back to it),
//! volume and brightness pop-ups, notifications of an app that was muted, and everything while
//! this computer's Do Not Disturb is on. A phone gets at most 20 a minute.
//!
//! Monitoring the bus is not allowed from the Flatpak's sandbox: the switch isn't offered there.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use linksy_core::{Event, Fingerprint, Node};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, info};
use zbus::zvariant::OwnedValue;

pub static OUTBOX: OnceLock<Outbox> = OnceLock::new();

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
/// The most a phone is sent in a minute.
const PER_MINUTE: usize = 20;
/// Linksy's own notifications, which carry this desktop entry.
const OWN_ENTRY: &str = "mg.diary.linksy.Linksy";

#[derive(Default, Serialize, Deserialize)]
struct Prefs {
    /// The phones that get this computer's notifications.
    #[serde(default)]
    phones: HashSet<Fingerprint>,
    /// Apps whose notifications stay here, by the name they notify with.
    #[serde(default)]
    muted: Vec<String>,
}

pub struct Outbox {
    path: PathBuf,
    prefs: Mutex<Prefs>,
    /// The apps seen lately, for choosing which to mute.
    seen: Mutex<VecDeque<String>>,
    /// Whether the bus lets this service listen.
    listening: std::sync::atomic::AtomicBool,
}

impl Outbox {
    pub fn load(path: PathBuf) -> Self {
        let prefs = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, prefs: Mutex::new(prefs), seen: Mutex::default(), listening: Default::default() }
    }

    fn prefs(&self) -> std::sync::MutexGuard<'_, Prefs> {
        self.prefs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, prefs: &Prefs) -> Result<(), String> {
        let bytes = serde_json::to_vec(prefs).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, bytes).map_err(|e| format!("could not save: {e}"))
    }

    pub fn info(&self, phone: &Fingerprint) -> linksy_ipc::ComputerNotifications {
        let prefs = self.prefs();
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
        linksy_ipc::ComputerNotifications { available: self.listening.load(std::sync::atomic::Ordering::Relaxed), on: prefs.phones.contains(phone), muted: prefs.muted.clone(), seen }
    }

    pub fn set(&self, phone: Fingerprint, on: bool) -> Result<(), String> {
        let mut prefs = self.prefs();
        if on { prefs.phones.insert(phone) } else { prefs.phones.remove(&phone) };
        self.save(&prefs)
    }

    pub fn mute(&self, app: &str, muted: bool) -> Result<(), String> {
        let mut prefs = self.prefs();
        prefs.muted.retain(|m| !m.eq_ignore_ascii_case(app));
        if muted {
            prefs.muted.push(app.to_string());
        }
        self.save(&prefs)
    }

    fn wanted(&self) -> Vec<Fingerprint> {
        self.prefs().phones.iter().copied().collect()
    }

    fn is_muted(&self, app: &str) -> bool {
        self.prefs().muted.iter().any(|m| m.eq_ignore_ascii_case(app))
    }

    fn note(&self, app: &str) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|a| a != app);
        seen.push_front(app.to_string());
        seen.truncate(30);
    }
}

/// What an application sent the notification server.
#[derive(Debug, PartialEq)]
pub struct Posted {
    pub app: String,
    pub title: String,
    pub body: String,
    /// Posted by Linksy, or a pop-up that isn't meant to be kept (volume, brightness).
    pub skip: bool,
}

/// Read a `Notify` call's arguments: `susssasa{sv}i`.
fn read(body: &zbus::message::Body) -> Option<Posted> {
    type Args = (String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32);
    let (app, _replaces, _icon, title, text, _actions, hints, _timeout): Args = body.deserialize().ok()?;
    let hint = |name: &str| hints.get(name).and_then(|v| <&str>::try_from(v).ok().map(String::from));
    let flag = |name: &str| hints.get(name).is_some_and(|v| bool::try_from(v).unwrap_or(true) || <&str>::try_from(v).is_ok());
    let own = hint("desktop-entry").is_some_and(|e| e == OWN_ENTRY || e == format!("{OWN_ENTRY}.desktop")) || app == "Linksy";
    // The pop-ups that replace each other as a slider moves
    let osd = flag("synchronous") || flag("x-canonical-private-synchronous") || flag("x-canonical-private-icon-only");
    Some(Posted { app, title, body: text, skip: own || osd })
}

pub async fn run(node: Node) {
    let Some(outbox) = OUTBOX.get() else { return };
    if linksy_host::flatpak() {
        return;
    }
    let Ok(builder) = zbus::connection::Builder::session() else { return };
    let Ok(bus) = builder.build().await else { return };
    let rules = vec![format!("type='method_call',interface='{NOTIFICATIONS}',member='Notify'")];
    let monitor = bus.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus.Monitoring"), "BecomeMonitor", &(rules, 0u32)).await;
    if let Err(e) = monitor {
        info!("this computer's notifications can't go to phones: the bus won't let Linksy listen ({e})");
        return;
    }
    outbox.listening.store(true, std::sync::atomic::Ordering::Relaxed);

    // To ask who the notification server is: the monitor can't send
    let Ok(caller) = zbus::Connection::session().await else { return };
    // Which phones can show them: connected, and announcing it
    let mut can: HashSet<Fingerprint> = node.devices().into_iter().filter(|d| d.connected && has(&d.device.features)).map(|d| d.device.id).collect();
    let mut events = node.subscribe();
    let mut sent: HashMap<Fingerprint, VecDeque<Instant>> = HashMap::new();
    let mut messages = zbus::MessageStream::from(&bus);
    let mut counter = 0u64;
    loop {
        tokio::select! {
            message = messages.next() => {
                let Some(Ok(message)) = message else { break };
                let header = message.header();
                if header.member().is_none_or(|m| m.as_str() != "Notify") {
                    continue;
                }

                let Some(posted) = read(&message.body()) else { continue };
                if posted.skip || (posted.title.is_empty() && posted.body.is_empty()) {
                    continue;
                }
                // The notification server passing one on to something else is not an app's
                let sender = header.sender().map(|s| s.to_string());
                if sender.is_some() && sender == server(&caller).await {
                    continue;
                }
                outbox.note(&posted.app);
                if outbox.is_muted(&posted.app) {
                    continue;
                }
                let phones: Vec<Fingerprint> = outbox.wanted().into_iter().filter(|p| can.contains(p)).collect();
                if phones.is_empty() {
                    continue;
                }
                // Held back while this computer holds its own back
                crate::controls::shared().probe().await;
                if crate::controls::shared().dnd().await == Some(true) {
                    continue;
                }
                counter += 1;
                let key = format!("{}-{counter}", std::process::id());
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                for phone in phones {
                    let recent = sent.entry(phone).or_default();
                    while recent.front().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
                        recent.pop_front();
                    }
                    if recent.len() >= PER_MINUTE {
                        debug!("outbox: too many for {}, dropped", phone.short());
                        continue;
                    }
                    recent.push_back(Instant::now());
                    let node = node.clone();
                    let (key, app, title, body) = (key.clone(), posted.app.clone(), posted.title.clone(), posted.body.clone());
                    tokio::spawn(async move {
                        if let Err(e) = node.send_computer_notification(&phone, &key, &app, &title, &body, at).await {
                            debug!("outbox: not sent: {e}");
                        }
                    });
                }
            }
            event = events.recv() => match event {
                Ok(Event::Connected { device }) if has(&device.features) => {
                    can.insert(device.id);
                }
                // A phone turning it on or off while connected
                Ok(Event::DeviceUpdated { device }) => {
                    if has(&device.features) && node.devices().iter().any(|d| d.connected && d.device.id == device.id) {
                        can.insert(device.id);
                    } else {
                        can.remove(&device.id);
                    }
                }
                Ok(Event::Disconnected { id }) => {
                    can.remove(&id);
                    sent.remove(&id);
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// The bus name that is the notification server now.
async fn server(caller: &zbus::Connection) -> Option<String> {
    let reply = tokio::time::timeout(Duration::from_secs(2), caller.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "GetNameOwner", &(NOTIFICATIONS,))).await.ok()?.ok()?;
    reply.body().deserialize::<String>().ok()
}

fn has(features: &[String]) -> bool {
    features.iter().any(|f| f == "computer_notifications")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outbox(name: &str) -> Outbox {
        let path = std::env::temp_dir().join(format!("linksy-outbox-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Outbox::load(path)
    }

    #[test]
    fn it_is_off_for_a_phone_until_turned_on_and_stays_so() {
        let o = outbox("on");
        let phone = Fingerprint([7; 32]);
        assert!(!o.info(&phone).on);
        o.set(phone, true).unwrap();
        assert!(o.info(&phone).on);
        let again = Outbox::load(o.path.clone());
        assert!(again.info(&phone).on);
        again.set(phone, false).unwrap();
        assert!(!Outbox::load(o.path.clone()).info(&phone).on);
        let _ = std::fs::remove_file(&o.path);
    }

    #[test]
    fn a_muted_app_is_muted_by_any_spelling_and_can_be_let_back() {
        let o = outbox("mute");
        o.mute("Thunderbird", true).unwrap();
        assert!(o.is_muted("thunderbird"));
        assert!(!o.is_muted("Firefox"));
        o.mute("THUNDERBIRD", false).unwrap();
        assert!(!o.is_muted("Thunderbird"));
        let _ = std::fs::remove_file(&o.path);
    }

    #[test]
    fn apps_seen_lately_are_listed_newest_first_without_repeats() {
        let o = outbox("seen");
        for app in ["a", "b", "a", "c"] {
            o.note(app);
        }
        let phone = Fingerprint([1; 32]);
        assert_eq!(o.info(&phone).seen, ["c", "a", "b"]);
    }
}
