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
    pub fn load(prefs_path: PathBuf, icons: PathBuf, socket: PathBuf) -> Self {
        let prefs = std::fs::read(&prefs_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let inner = Inner { prefs_path, prefs: Mutex::new(prefs), shown: Mutex::default(), icons, socket, next: Default::default() };
        Self { inner: Arc::new(inner) }
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
                    if prefs.show && !prefs.muted.iter().any(|m| m.app == notification.app) {
                        self.show(node.clone(), from, notification);
                    }
                }
                Ok(Event::NotificationRemoved { from, key }) => {
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
    fn show(&self, node: Node, from: DeviceInfo, n: RemoteNotification) {
        use notify_rust::{CloseReason, NotificationResponse};

        let entry = (from.id, n.key.clone());
        let generation = self.inner.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let previous = {
            let mut shown = self.inner.shown.lock().unwrap();
            let previous = shown.get(&entry).map(|s| s.id).unwrap_or(0);
            shown.insert(entry.clone(), Shown { from: from.clone(), notification: n.clone(), id: previous, generation });
            previous
        };
        let icon = n.icon.as_deref().and_then(|b64| self.unpack_icon(&n.app, b64));
        let mirror = self.clone();
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let body = if n.body.is_empty() { format!("{} · {}", n.app_name, from.name) } else { format!("{}\n{} · {}", n.body, n.app_name, from.name) };
            let mut note = crate::notify::base(&n.title, &body);
            if let Some(icon) = &icon {
                note.icon(&icon.to_string_lossy());
            }
            if previous != 0 {
                // Same notification, new content: update it in place.
                note.id(previous);
            }
            if n.can_reply {
                note.action("reply", "Reply");
            }
            if let Some(b) = n.actions.first() {
                note.action(&format!("act:{}", b.id), &b.label);
            }
            note.action("mute", &format!("Mute {}", n.app_name));
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
                    NotificationResponse::Action(a) if a == "mute" => {
                        mirror.mute(&n.app, &n.app_name);
                        mirror.inner.shown.lock().unwrap().remove(&entry);
                    }
                    NotificationResponse::Action(a) if a.starts_with("act:") => {
                        if let Err(e) = rt.block_on(node.notification_action(from, key, &a[4..])) {
                            warn!("notification button failed: {e}");
                        }
                    }
                    // Clicked or swiped away: the user has seen it, so it's done on the phone too.
                    NotificationResponse::Default | NotificationResponse::Closed(CloseReason::Dismissed) => {
                        mirror.inner.shown.lock().unwrap().remove(&entry);
                        if let Err(e) = rt.block_on(node.dismiss_notification(from, key)) {
                            warn!("dismissing on the phone failed: {e}");
                        }
                    }
                    _ => {
                        mirror.inner.shown.lock().unwrap().remove(&entry);
                    }
                }
            });
        });
    }

    /// Write an app's icon (base64 PNG) where the notification server can read it.
    #[cfg(target_os = "linux")]
    fn unpack_icon(&self, app: &str, b64: &str) -> Option<PathBuf> {
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        if !png.starts_with(b"\x89PNG") {
            return None;
        }
        let name: String = app.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
        let path = self.inner.icons.join(format!("{name}.png"));
        if std::fs::read(&path).ok().as_deref() != Some(&png[..]) {
            std::fs::create_dir_all(&self.inner.icons).ok()?;
            std::fs::write(&path, &png).ok()?;
        }
        Some(path)
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
