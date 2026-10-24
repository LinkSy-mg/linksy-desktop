//! The tray icon, from the service: one for every desktop that shows tray icons (KDE Plasma,
//! GNOME with the AppIndicator extension, most others) and for the Flatpak, without keeping
//! a window app open for it. Its menu does what a device's page does most (the menu and
//! icons are yurividal's, from the KDE app), and Show Linksy opens the window for the rest.
//!
//! Where nothing shows tray icons, it waits quietly and appears once something does.

use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{MenuItem, TrayMethods};
use linksy_core::{DeviceInfo, DeviceKind, DeviceView, Event, Node, SessionKind, Uuid};
use linksy_i18n::t;
use linksy_ipc::{ActiveSession, Request};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::history::History;
use crate::lock::Away;
use crate::mirror::Mirror;
use crate::photos::Photos;
use crate::sessions::Sessions;

/// What the service needs to act on the menu: the same calls the apps make.
#[derive(Clone)]
pub struct Service {
    pub node: Node,
    pub history: History,
    pub sessions: Sessions,
    pub mirror: Mirror,
    pub away: Away,
    pub photos: Photos,
    pub socket: PathBuf,
}

impl Service {
    async fn ask(&self, req: Request) -> Result<Value, String> {
        crate::ipc::handle(&self.node, &self.history, &self.sessions, &self.mirror, &self.away, &self.photos, req).await
    }
}

/// What the menu shows: paired devices and what's being shared.
#[derive(Clone, Default)]
struct Shown {
    devices: Vec<DeviceView>,
    sessions: Vec<ActiveSession>,
}

// Compared as they're sent to the apps
impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        let json = |s: &Self| serde_json::to_string(&(&s.devices, &s.sessions)).unwrap_or_default();
        json(self) == json(other)
    }
}

/// A menu choice, done by the service (menu callbacks can't wait).
enum Action {
    Open,
    SendFiles(DeviceInfo),
    Ring(DeviceInfo),
    Ping(DeviceInfo),
    Locate(DeviceInfo),
    Browse(DeviceInfo),
    Start(DeviceInfo, SessionKind),
    Stop(Uuid),
    Quit,
}

struct Tray {
    shown: Shown,
    actions: mpsc::UnboundedSender<Action>,
}

pub async fn run(service: Service) {
    let (actions, mut todo) = mpsc::unbounded_channel();
    let tray = Tray { shown: shown(&service), actions };
    // The Flatpak can't own the usual D-Bus name; a desktop without a tray yet may get one
    let sandboxed = std::env::var_os("FLATPAK_ID").is_some();
    let handle = match tray.disable_dbus_name(sandboxed).assume_sni_available(true).spawn().await {
        Ok(h) => h,
        Err(e) => {
            info!("no tray icon: {e}");
            return;
        }
    };
    let mut events = service.node.subscribe();
    let mut last = shown(&service);
    // Forgetting a device or a helper ending send no event of their own
    let mut every = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            Some(action) = todo.recv() => {
                let service = service.clone();
                tokio::spawn(async move { act(&service, action).await });
            }
            event = events.recv() => match event {
                Ok(Event::DeviceFound { .. } | Event::DeviceLost { .. } | Event::Connected { .. } | Event::Disconnected { .. }
                    | Event::Paired { .. } | Event::DeviceUpdated { .. } | Event::SessionStarted { .. } | Event::SessionEnded { .. })
                | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Several come together (a device connecting): once they settle
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    refresh(&service, &handle, &mut last).await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = every.tick() => refresh(&service, &handle, &mut last).await,
        }
    }
}

async fn refresh(service: &Service, handle: &ksni::Handle<Tray>, last: &mut Shown) {
    let now = shown(service);
    if now != *last {
        *last = now.clone();
        handle.update(|t| t.shown = now).await;
    }
}

fn shown(service: &Service) -> Shown {
    let mut devices: Vec<DeviceView> = service.node.devices().into_iter().filter(|d| d.trusted).collect();
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.device.name.to_lowercase().cmp(&b.device.name.to_lowercase())));
    Shown { devices, sessions: service.sessions.active() }
}

async fn act(service: &Service, action: Action) {
    let (who, result) = match action {
        Action::Open => {
            crate::sessions::open_window(&service.socket);
            return;
        }
        Action::Quit => (None, service.ask(Request::Quit).await),
        Action::SendFiles(d) => match pick_files(&t!("Send to {name}", name = d.name)).await {
            Some(paths) if !paths.is_empty() => (Some(d.name), service.ask(Request::Send { to: d.id.to_string(), paths }).await),
            _ => return,
        },
        Action::Ring(d) => (Some(d.name), service.ask(Request::Ring { to: d.id.to_string(), on: true }).await),
        Action::Ping(d) => (Some(d.name), service.ask(Request::Notify { to: d.id.to_string(), title: "Ping".into(), body: String::new() }).await),
        Action::Locate(d) => {
            let found = service.ask(Request::Locate { device: d.id.to_string() }).await;
            if let Ok(at) = &found {
                let (lat, lon) = (at["lat"].as_f64().unwrap_or(0.0), at["lon"].as_f64().unwrap_or(0.0));
                open(&format!("https://www.openstreetmap.org/?mlat={lat:.5}&mlon={lon:.5}#map=17/{lat:.5}/{lon:.5}"));
            }
            (Some(d.name), found)
        }
        Action::Browse(d) => {
            let found = service.ask(Request::BrowseFiles { phone: d.id.to_string() }).await;
            if let Ok(Value::String(at)) = &found {
                open(&if at.starts_with('/') { format!("file://{at}") } else { at.clone() });
            }
            (Some(d.name), found)
        }
        Action::Start(d, kind) => (Some(d.name), service.ask(Request::StartSession { with: d.id.to_string(), kind }).await),
        Action::Stop(session) => (None, service.ask(Request::EndSession { session }).await),
    };
    // Nowhere else to say it went wrong: a notification
    if let Err(e) = result {
        crate::notify::show(who.unwrap_or_else(|| "Linksy".into()), e);
    }
}

fn open(target: &str) {
    if let Err(e) = tokio::process::Command::new("xdg-open").arg(target).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        debug!("xdg-open: {e}");
    }
}

/// Files chosen in the desktop's own file dialog (the portal's, which works from a
/// service and from the Flatpak). None when cancelled.
async fn pick_files(title: &str) -> Option<Vec<PathBuf>> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};
    let bus = zbus::Connection::session().await.ok()?;
    // The portal answers on a path made of our name and this token: listen there first
    let token = format!("linksy{}", std::process::id() ^ (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.subsec_nanos()));
    let sender = bus.unique_name()?.trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.portal.Request").ok()?.member("Response").ok()?.path(path.as_str()).ok()?.build();
    let mut answers = zbus::MessageStream::for_match_rule(rule, &bus, Some(1)).await.ok()?;
    let options: HashMap<&str, Value> = HashMap::from([("handle_token", Value::from(token.as_str())), ("multiple", Value::from(true))]);
    bus.call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.FileChooser"), "OpenFile", &("", title, options))
        .await
        .map_err(|e| debug!("file dialog: {e}"))
        .ok()?;
    let answer = answers.next().await?.ok()?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = answer.body().deserialize().ok()?;
    if code != 0 {
        return None;
    }
    let uris: Vec<String> = results.get("uris")?.try_clone().ok()?.try_into().ok()?;
    Some(uris.iter().filter_map(|u| u.strip_prefix("file://")).map(|p| PathBuf::from(crate::media::percent_decode(p))).collect())
}

/// Whether a device can do something: what it says, or for an older phone that says
/// nothing, all but what came later (as the apps decide).
fn can(info: &DeviceInfo, feature: &str) -> bool {
    if !info.features.is_empty() {
        return info.features.iter().any(|f| f == feature);
    }
    !(info.os == "android" && ["microphone", "locate"].contains(&feature))
}

fn session_title(s: &ActiveSession) -> String {
    let name = s.peer.name.as_str();
    match s.kind {
        SessionKind::Camera if s.sharing => t!("{name} is using your camera", name),
        SessionKind::Camera => t!("{name}'s camera is on: “{camera}” in your apps", name, camera = format!("Linksy Camera ({name})")),
        SessionKind::Microphone if s.sharing => t!("{name} is using your microphone", name),
        SessionKind::Microphone => t!("{name}'s microphone is on: “{microphone}” in your apps", name, microphone = format!("Linksy Microphone ({name})")),
        SessionKind::Display => t!("{name} is your second display", name),
        SessionKind::Control => t!("{name} is controlling your screen", name),
        SessionKind::Input => t!("{name} is your touchpad and keyboard", name),
        SessionKind::Screen => t!("{name} is watching your screen", name),
        SessionKind::Terminal => t!("{name} has a terminal open here", name),
    }
}

/// A menu entry that sends `action` (made again on each click: the menu is rebuilt often).
/// Icons are symbolic ones, one colour like the menu's text: the plain names are full-colour
/// pictures on GNOME.
fn item(label: String, icon: &str, enabled: bool, action: impl Fn() -> Action + Send + Sync + 'static) -> MenuItem<Tray> {
    StandardItem {
        label,
        icon_name: icon.into(),
        enabled,
        activate: Box::new(move |t: &mut Tray| {
            let _ = t.actions.send(action());
        }),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "linksy".into()
    }

    fn title(&self) -> String {
        "Linksy".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }

    // Filled in while a device is connected, hollow and dim otherwise
    fn icon_name(&self) -> String {
        if self.connected() > 0 { "mg.diary.linksy.Linksy-tray-symbolic" } else { "mg.diary.linksy.Linksy-tray-idle-symbolic" }.into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Linksy".into(), description: self.status(), ..Default::default() }
    }

    // A click opens the window; the menu is on the other button
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.actions.send(Action::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = vec![StandardItem { label: self.status(), enabled: false, ..Default::default() }.into(), MenuItem::Separator];

        // Each paired device: what its page offers most
        for d in &self.shown.devices {
            let (info, on) = (&d.device, d.connected);
            let phone = info.kind == DeviceKind::Phone;
            let mut sub = Vec::new();
            let with = |f: fn(DeviceInfo) -> Action| {
                let info = info.clone();
                move || f(info.clone())
            };
            sub.push(item(t!("Send Files").to_string(), "document-send-symbolic", on, with(Action::SendFiles)));
            if phone && can(info, "ring") {
                sub.push(item(t!("Ring").to_string(), "audio-volume-high-symbolic", on, with(Action::Ring)));
            } else {
                sub.push(item(t!("Ping").to_string(), "preferences-system-notifications-symbolic", on, with(Action::Ping)));
            }
            if phone && can(info, "locate") {
                sub.push(item(t!("Locate").to_string(), "find-location-symbolic", on, with(Action::Locate)));
            }
            if phone && can(info, "fs") {
                sub.push(item(t!("Browse Files").to_string(), "folder-open-symbolic", on, with(Action::Browse)));
            }
            // What it can be for this computer: checked while it is, a click starts or stops it
            let uses = [(SessionKind::Display, "display", t!("Second Display"), "video-display-symbolic"), (SessionKind::Camera, "camera", t!("Webcam"), "camera-web-symbolic"), (SessionKind::Microphone, "microphone", t!("Microphone"), "audio-input-microphone-symbolic")];
            let uses: Vec<_> = uses.into_iter().filter(|u| can(info, u.1)).collect();
            if !uses.is_empty() {
                sub.push(MenuItem::Separator);
            }
            for (kind, _, label, icon) in uses {
                let running = self.shown.sessions.iter().find(|s| s.kind == kind && s.peer.id == info.id).map(|s| s.session);
                let info = info.clone();
                sub.push(
                    CheckmarkItem {
                        label: label.into(),
                        icon_name: icon.into(),
                        enabled: on || running.is_some(),
                        checked: running.is_some(),
                        activate: Box::new(move |t: &mut Tray| {
                            let _ = t.actions.send(match running {
                                Some(session) => Action::Stop(session),
                                None => Action::Start(info.clone(), kind),
                            });
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            let icon = match info.kind {
                DeviceKind::Phone => "phone-symbolic",
                DeviceKind::Tablet => "tablet-symbolic",
                DeviceKind::Laptop => "computer-symbolic",
                DeviceKind::Desktop => "computer-symbolic",
            };
            menu.push(SubMenu { label: info.name.clone(), icon_name: icon.into(), submenu: sub, ..Default::default() }.into());
        }
        if !self.shown.devices.is_empty() {
            menu.push(MenuItem::Separator);
        }

        for s in &self.shown.sessions {
            let session = s.session;
            menu.push(item(t!("Stop: {what}", what = session_title(s)), "media-playback-stop-symbolic", true, move || Action::Stop(session)));
        }
        if !self.shown.sessions.is_empty() {
            menu.push(MenuItem::Separator);
        }

        menu.push(item(t!("Show Linksy").to_string(), "mg.diary.linksy.Linksy-symbolic", true, || Action::Open));
        menu.push(item(t!("Quit").to_string(), "application-exit-symbolic", true, || Action::Quit));
        menu
    }
}

impl Tray {
    fn connected(&self) -> usize {
        self.shown.devices.iter().filter(|d| d.connected).count()
    }

    fn status(&self) -> String {
        match self.connected() {
            0 => t!("No devices connected").to_string(),
            1 => t!("1 device connected").to_string(),
            n => t!("{n} devices connected", n),
        }
    }
}
