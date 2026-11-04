//! Phone notifications on this computer. Each one a phone mirrors shows as a native
//! notification with Reply (when the app allows it), the phone's own first button, and
//! Mute; dismissing it here dismisses it on the phone, and it goes away here when it goes
//! away there. Nothing about them is written to disk except which apps are muted.
//! Other systems keep the state (so IPC answers) but show nothing yet.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use linksy_core::{DeviceInfo, Fingerprint, Node, RemoteNotification};
use linksy_i18n::t;
use linksy_ipc::{MutedApp, NotificationPrefs};
use tracing::warn;

#[derive(Clone)]
pub struct Mirror {
    inner: Arc<Inner>,
}

struct Inner {
    prefs_path: PathBuf,
    prefs: Mutex<NotificationPrefs>,
    /// Notifications on screen, by phone and key.
    shown: Mutex<HashMap<(Fingerprint, String), Shown>>,
    /// Where app icons are unpacked for the notification server to read.
    icons: PathBuf,
    /// This service's IPC socket, for the reply window it opens.
    socket: PathBuf,
    next: std::sync::atomic::AtomicU64,
    inbox: crate::inbox::Inbox,
    /// Shown on the desktop (not with --no-notifications); the inbox fills either way.
    popups: bool,
}

struct Shown {
    from: DeviceInfo,
    notification: RemoteNotification,
    /// This desktop's id for it (0 until shown), to update or close it.
    id: u32,
    /// Which showing of the key this is; a newer one supersedes the older's buttons.
    generation: u64,
}

impl Mirror {
    pub fn load(prefs_path: PathBuf, icons: PathBuf, socket: PathBuf, popups: bool) -> Self {
        let prefs = std::fs::read(&prefs_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let inbox = crate::inbox::Inbox::new(icons.clone());
        let inner = Inner { prefs_path, prefs: Mutex::new(prefs), shown: Mutex::default(), icons, socket, next: Default::default(), inbox, popups };
        Self { inner: Arc::new(inner) }
    }

    pub fn inbox(&self) -> &crate::inbox::Inbox {
        &self.inner.inbox
    }

    /// Conversation `id` read in the app: its notification here goes too.
    pub async fn read(&self, id: &str) {
        self.inner.inbox.read(id);
        let Some((phone, key, _)) = self.inner.inbox.notification(id) else { return };
        let gone = self.inner.shown.lock().unwrap().remove(&(phone, key));
        if let Some(s) = gone {
            close(s.id).await;
        }
    }

    /// Dismiss `phone`'s other notification `key` there and here, or all of them.
    pub async fn dismiss(&self, node: &Node, phone: &Fingerprint, key: Option<&str>) -> Result<(), String> {
        let keys = match key {
            Some(k) => vec![k.to_string()],
            None => self.inner.inbox.alert_keys(phone),
        };
        for key in keys {
            node.dismiss_notification(phone, &key).await.map_err(|e| e.to_string())?;
            self.inner.inbox.removed(phone, &key, false);
            let gone = self.inner.shown.lock().unwrap().remove(&(*phone, key));
            if let Some(s) = gone {
                close(s.id).await;
            }
        }
        Ok(())
    }

    /// Conversation `id`'s earlier messages, asked of the phone: the whole conversation then.
    pub async fn older(&self, node: &Node, id: &str) -> Result<Vec<linksy_ipc::InboxMessage>, String> {
        use linksy_core::Event;
        use tokio::sync::broadcast::error::RecvError;

        let (phone, app, conversation, before) = self.inner.inbox.older_than(id).ok_or("no earlier messages to ask for")?;
        let mut events = node.subscribe();
        node.inbox_history(&phone, &app, &conversation, before, 50).await.map_err(|e| e.to_string())?;
        let page = async {
            loop {
                match events.recv().await {
                    Ok(Event::InboxHistoryPage { from, app: a, conversation: c, messages, more }) if from.id == phone && a == app && c == conversation => return Some((messages, more)),
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return None,
                }
            }
        };
        let (messages, more) = tokio::time::timeout(std::time::Duration::from_secs(10), page).await.ok().flatten().ok_or("the phone didn't answer")?;
        self.inner.inbox.add_older(&phone, &app, &conversation, &messages, more);
        self.inner.inbox.thread(id).ok_or_else(|| "that conversation is gone".into())
    }

    /// Answer conversation `id` through the phone's notification.
    pub async fn reply_in(&self, node: &Node, id: &str, text: &str) -> Result<(), String> {
        let (phone, key, can_reply) = self.inner.inbox.notification(id).ok_or("answer it on the phone: its notification is gone")?;
        if !can_reply {
            return Err("this app's notification can't be answered from here".into());
        }
        self.reply(node, &phone, &key, text).await?;
        self.inner.inbox.replied(id, text);
        Ok(())
    }

    pub fn prefs(&self) -> NotificationPrefs {
        self.inner.prefs.lock().unwrap().clone()
    }

    pub fn set_prefs(&self, prefs: NotificationPrefs) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&prefs).map_err(std::io::Error::other)?;
        std::fs::write(&self.inner.prefs_path, json)?;
        *self.inner.prefs.lock().unwrap() = prefs;
        Ok(())
    }

    fn mute(&self, app: &str, app_name: &str) {
        let mut prefs = self.prefs();
        if !prefs.muted.iter().any(|m| m.app == app) {
            prefs.muted.push(MutedApp { app: app.into(), app_name: app_name.into() });
            if let Err(e) = self.set_prefs(prefs) {
                warn!("could not save notification settings: {e}");
            }
        }
    }

    /// A notification shown here, for the reply window.
    pub fn get(&self, from: &Fingerprint, key: &str) -> Option<(DeviceInfo, RemoteNotification)> {
        let shown = self.inner.shown.lock().unwrap();
        shown.get(&(*from, key.to_string())).map(|s| (s.from.clone(), s.notification.clone()))
    }

    /// Send a reply to the phone, and take the notification down here.
    pub async fn reply(&self, node: &Node, from: &Fingerprint, key: &str, text: &str) -> Result<(), String> {
        node.reply_notification(from, key, text).await.map_err(|e| e.to_string())?;
        let gone = self.inner.shown.lock().unwrap().remove(&(*from, key.to_string()));
        if let Some(s) = gone {
            close(s.id).await;
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn run(self, _node: Node) {}

    #[cfg(target_os = "linux")]
    pub async fn run(self, node: Node) {
        use linksy_core::Event;
        use tokio::sync::broadcast::error::RecvError;

        let mut events = node.subscribe();
        loop {
            match events.recv().await {
                Ok(Event::NotificationPosted { from, notification }) => {
                    let prefs = self.prefs();
                    if prefs.muted.iter().any(|m| m.app == notification.app) {
                        continue;
                    }
                    let code = self.inner.inbox.posted(&from, &notification);
                    let copied = code.as_deref().filter(|_| prefs.copy_codes).is_some_and(|c| crate::clipboard::copy_text(c).is_ok());
                    if prefs.show && self.inner.popups {
                        self.show(node.clone(), from, *notification, code, copied);
                    }
                }
                Ok(Event::NotificationRemoved { from, key, reply_kept }) => {
                    self.inner.inbox.removed(&from.id, &key, reply_kept);
                    let gone = self.inner.shown.lock().unwrap().remove(&(from.id, key));
                    if let Some(s) = gone {
                        close(s.id).await;
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn show(&self, node: Node, from: DeviceInfo, n: RemoteNotification, code: Option<String>, copied: bool) {
        use notify_rust::{CloseReason, NotificationResponse};

        let entry = (from.id, n.key.clone());
        let generation = self.inner.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let previous = {
            let mut shown = self.inner.shown.lock().unwrap();
            let previous = shown.get(&entry).map(|s| s.id).unwrap_or(0);
            shown.insert(entry.clone(), Shown { from: from.clone(), notification: n.clone(), id: previous, generation });
            previous
        };
        let icon = n.icon.as_deref().and_then(|b64| crate::inbox::unpack(&self.inner.icons, &n.app, b64));
        let several = node.trusted().iter().filter(|t| t.device.kind == linksy_core::DeviceKind::Phone).count() > 1;
        let mirror = self.clone();
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let summary = summary(&n.title, &n.app_name, several.then_some(from.name.as_str()), copied);
            let mut note = crate::notify::base(&summary, &n.body);
            if let Some(icon) = &icon {
                note.icon(&icon.to_string_lossy());
            }
            if previous != 0 {
                // Same notification, new content: update it in place.
                note.id(previous);
            }
            if let Some(code) = &code {
                note.action(&format!("copy:{code}"), &t!("Copy {code}", code));
            }
            if n.can_reply {
                note.action("reply", t!("Reply"));
            }
            if let Some(b) = n.actions.first().filter(|_| code.is_none() || !n.can_reply) {
                note.action(&format!("act:{}", b.id), &b.label);
            }
            note.action("mute", &t!("Mute {app}", app = n.app_name));
            // The desktop may have dropped the one to update; then it's a new one.
            let shown = note.show().or_else(|e| if previous != 0 { note.id(0).show() } else { Err(e) });
            let handle = match shown {
                Ok(h) => h,
                Err(e) => {
                    warn!("notification failed: {e}");
                    return;
                }
            };
            let id = handle.id();
            // Superseded or removed while it was being shown: nothing more to do.
            let current = |m: &Mirror| m.inner.shown.lock().unwrap().get(&entry).is_some_and(|s| s.generation == generation);
            match mirror.inner.shown.lock().unwrap().get_mut(&entry) {
                Some(s) if s.generation == generation => s.id = id,
                _ => return,
            }
            let _ = handle.wait_for_response(|response: &NotificationResponse| {
                if !current(&mirror) {
                    return;
                }
                let (from, key) = (&entry.0, entry.1.as_str());
                match response {
                    // The reply window takes it from here (and takes it down when sent).
                    NotificationResponse::Action(a) if a == "reply" => {
                        let opened = std::process::Command::new(crate::sessions::helper("linksy-app"))
                            .args(["--reply", &from.to_string(), key])
                            .env("LINKSY_SOCKET", &mirror.inner.socket)
                            .stdin(std::process::Stdio::null())
                            .spawn();
                        if let Err(e) = opened {
                            warn!("could not open the reply window: {e}");
                        }
                    }
                    NotificationResponse::Action(a) if a.starts_with("copy:") => {
                        let _ = crate::clipboard::copy_text(&a[5..]);
                    }
                    NotificationResponse::Action(a) if a == "mute" => {
                        mirror.mute(&n.app, &n.app_name);
                        mirror.inner.shown.lock().unwrap().remove(&entry);
                    }
                    // Spawned, not blocked on: this runs inside the notification library's own
                    // runtime, and blocking there aborted the service
                    NotificationResponse::Action(a) if a.starts_with("act:") => {
                        let (node, from, key, action) = (node.clone(), *from, key.to_string(), a[4..].to_string());
                        rt.spawn(async move {
                            if let Err(e) = node.notification_action(&from, &key, &action).await {
                                warn!("notification button failed: {e}");
                            }
                        });
                    }
                    // Clicked or swiped away: the user has seen it, so it's done on the phone too.
                    NotificationResponse::Default | NotificationResponse::Closed(CloseReason::Dismissed) => {
                        mirror.inner.shown.lock().unwrap().remove(&entry);
                        let (node, from, key) = (node.clone(), *from, key.to_string());
                        rt.spawn(async move {
                            if let Err(e) = node.dismiss_notification(&from, &key).await {
                                warn!("dismissing on the phone failed: {e}");
                            }
                        });
                    }
                    _ => {
                        mirror.inner.shown.lock().unwrap().remove(&entry);
                    }
                }
            });
        });
    }
}

/// Take a notification off the screen by its id.
#[cfg(target_os = "linux")]
async fn close(id: u32) {
    if id == 0 {
        return;
    }
    let closed = async {
        let bus = zbus::Connection::session().await?;
        bus.call_method(Some("org.freedesktop.Notifications"), "/org/freedesktop/Notifications", Some("org.freedesktop.Notifications"), "CloseNotification", &(id,))
            .await?;
        Ok::<_, zbus::Error>(())
    };
    if let Err(e) = closed.await {
        tracing::debug!("closing notification {id}: {e}");
    }
}

#[cfg(not(target_os = "linux"))]
async fn close(_id: u32) {}

/// A phone notification's title here, with where it's from ("Jenna · WhatsApp"): desktops
/// show the body on one line, where a line about the app would read as part of the message.
/// `device` when there's more than one phone.
fn summary(title: &str, app: &str, device: Option<&str>, copied: bool) -> String {
    let title = title.trim();
    let mut parts = vec![title];
    if title != app {
        parts.push(app);
    }
    parts.extend(device);
    parts.retain(|p| !p.trim().is_empty());
    let summary = parts.join(" · ");
    if copied { t!("{what} · code copied", what = summary) } else { summary }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn says_where_from_in_the_title() {
        assert_eq!(summary("Jenna", "WhatsApp", None, false), "Jenna · WhatsApp");
        assert_eq!(summary("Jenna", "WhatsApp", Some("realme 15 Pro 5G"), false), "Jenna · WhatsApp · realme 15 Pro 5G");
        assert_eq!(summary("WhatsApp", "WhatsApp", None, false), "WhatsApp");
        assert_eq!(summary(" ", "Bank", None, true), "Bank · code copied");
    }
}
