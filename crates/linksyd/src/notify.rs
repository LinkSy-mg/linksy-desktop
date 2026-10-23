//! Native desktop notifications for things that need the user, even with no window open.
//! Consent prompts carry Accept/Decline buttons; whichever answer comes first (here or in
//! the app) wins, and the prompt goes away either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use linksy_core::{CallState, Direction, Event, Fingerprint, Node, SessionKind, Uuid};
use linksy_i18n::t;
use notify_rust::{Hint, Notification, Timeout, Urgency};
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

const APP: &str = "Linksy";
pub(crate) const DESKTOP_ENTRY: &str = "mg.diary.linksy.Linksy";
/// Battery level, in percent, at which a device's low battery is announced (once).
const LOW_BATTERY: u8 = 15;

/// Prompts on screen: request → the notification's id, and for pairing the device asking.
type Prompts = HashMap<Uuid, (u32, Option<Fingerprint>)>;
static PROMPTS: Mutex<Option<Prompts>> = Mutex::new(None);

/// A request was answered, here or anywhere else: take its prompt down.
pub(crate) fn answered(request: Uuid) {
    let id = PROMPTS.lock().unwrap().as_mut().and_then(|p| p.remove(&request)).map(|(id, _)| id);
    if let Some(id) = id {
        tokio::spawn(close(id));
    }
}

/// Pairing with `device` ended (paired, or failed): take its prompt down.
fn pairing_over(device: Fingerprint) {
    let ids: Vec<u32> = match PROMPTS.lock().unwrap().as_mut() {
        Some(p) => {
            let done: Vec<Uuid> = p.iter().filter(|(_, (_, d))| *d == Some(device)).map(|(r, _)| *r).collect();
            done.iter().filter_map(|r| p.remove(r)).map(|(id, _)| id).collect()
        }
        None => Vec::new(),
    };
    for id in ids {
        tokio::spawn(close(id));
    }
}

async fn close(id: u32) {
    let Ok(bus) = zbus::Connection::session().await else { return };
    let closed = bus
        .call_method(Some("org.freedesktop.Notifications"), "/org/freedesktop/Notifications", Some("org.freedesktop.Notifications"), "CloseNotification", &(id,))
        .await;
    if let Err(e) = closed {
        debug!("couldn't close notification {id}: {e}");
    }
}

pub async fn run(node: Node) {
    let mut events = node.subscribe();
    // Incoming transfers: remember who sent what so completion can say it.
    let mut incoming: std::collections::HashMap<Uuid, (String, bool)> = Default::default();
    // Devices already warned about a low battery; cleared once they charge or recover.
    let mut warned: std::collections::HashSet<Fingerprint> = Default::default();
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
        match event {
            Event::PairingRequest { request, device, code, .. } => {
                let body = t!("Check that {device} shows the code {code}.", device = device.name, code = spaced(&code));
                prompt(node.clone(), request, Some(device.id), t!("Pair with {device}?", device = device.name), body, t!("Pair"));
            }
            Event::TransferRequest { transfer, from, files, total } => {
                let what = match files.as_slice() {
                    [one] => display_name(&one.path),
                    many => t!("{n} items", n = many.len()),
                };
                let body = format!("{what} · {}", human_size(total));
                prompt(node.clone(), transfer, None, t!("{device} wants to send you files", device = from.name), body, t!("Accept"));
            }
            Event::SessionRequest { request, from, kind } => {
                let (summary, body) = match kind {
                    SessionKind::Control => (
                        t!("{device} wants to control your screen", device = from.name),
                        t!("{device} will see your screen and use your mouse and keyboard until you stop.", device = from.name),
                    ),
                    SessionKind::Camera => (
                        t!("{device} wants to use your camera", device = from.name),
                        t!("Your camera will be on and {device} will see it until you stop.", device = from.name),
                    ),
                    SessionKind::Display => (
                        t!("{device} wants to use this screen as a second display", device = from.name),
                        t!("Linksy will fill this screen with {device}'s second display, and your mouse and keyboard here will work on it.", device = from.name),
                    ),
                    SessionKind::Microphone => (
                        t!("{device} wants to use your microphone", device = from.name),
                        t!("Your microphone will be on and {device} will hear it until you stop.", device = from.name),
                    ),
                    SessionKind::Input => (
                        t!("{device} wants to use your mouse and keyboard", device = from.name),
                        t!("{device} becomes a touchpad and keyboard for this computer until you stop. It doesn't see your screen.", device = from.name),
                    ),
                    SessionKind::Screen => (
                        t!("{device} wants to see your screen", device = from.name),
                        t!("{device} will see everything on your screen until you stop sharing.", device = from.name),
                    ),
                    SessionKind::Terminal => (
                        t!("{device} wants a terminal on this computer", device = from.name),
                        t!("The right code was typed on {device}. Its terminal runs commands as you until it's ended. Linksy asks once a day.", device = from.name),
                    ),
                };
                prompt(node.clone(), request, None, summary, body, t!("Allow"));
            }
            Event::TransferStarted { transfer, peer, direction: Direction::Incoming, import, .. } => {
                answered(transfer);
                incoming.insert(transfer, (peer.name, import));
            }
            Event::TransferComplete { transfer, saved } => match incoming.remove(&transfer) {
                Some((from, true)) => imported(from, saved),
                Some((from, false)) => received(from, saved),
                None => {}
            },
            Event::TransferFailed { transfer, reason } => {
                answered(transfer);
                if let Some((from, _)) = incoming.remove(&transfer)
                    && reason != "cancelled"
                {
                    show(t!("Transfer from {device} failed", device = from), capitalize(&reason));
                }
            }
            Event::SessionStarted { session, .. } | Event::SessionEnded { session } => answered(session),
            Event::PairingFailed { device, .. } => pairing_over(device.id),
            Event::Paired { device } => {
                pairing_over(device.id);
                show(t!("Paired with {device}", device = device.name), t!("You can now share files and more.").into());
            }
            Event::Notification { from, title, body } => {
                let from = t!("From {device}", device = from.name);
                let body = if body.is_empty() { from } else { format!("{body}\n{from}") };
                show(title, body);
            }
            Event::Battery { from, battery } if battery.level <= LOW_BATTERY && !battery.charging => {
                if warned.insert(from.id) {
                    show(t!("{device} is low on battery", device = from.name), t!("{level}% left", level = battery.level));
                }
            }
            Event::Battery { from, battery } if battery.charging || battery.level > LOW_BATTERY + 5 => {
                warned.remove(&from.id);
            }
            Event::CallState { from, state: CallState::Ringing, contact } => {
                let summary = match contact {
                    Some(c) => t!("Call from {contact}", contact = c),
                    None => t!("Incoming call").to_string(),
                };
                tokio::task::spawn_blocking(move || {
                    let mut n = base(&summary, &t!("On {device}", device = from.name));
                    n.hint(Hint::Transient(true)).timeout(Timeout::Milliseconds(20_000));
                    if let Err(e) = n.show() {
                        debug!("notification failed: {e}");
                    }
                });
            }
            Event::Ring { from, on: true } => {
                tokio::task::spawn_blocking(move || {
                    let mut n = base(&t!("{device} is looking for this computer", device = from.name), t!("Here it is!"));
                    n.urgency(Urgency::Critical).hint(Hint::SoundName("phone-incoming-call".into()));
                    if let Err(e) = n.show() {
                        debug!("notification failed: {e}");
                    }
                });
            }
            _ => {}
        }
    }
}

pub fn base(summary: &str, body: &str) -> Notification {
    let mut n = Notification::new();
    n.appname(APP).summary(summary).body(body).icon(DESKTOP_ENTRY).hint(Hint::DesktopEntry(DESKTOP_ENTRY.into()));
    n
}

pub(crate) fn show(summary: String, body: String) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = base(&summary, &body).show() {
            debug!("notification failed: {e}");
        }
    });
}

/// A notification with Accept/Decline that answers the request when clicked. It stays until
/// answered (critical, no timeout), and goes when it is: clicking a button closes it (it
/// isn't resident), and an answer from the app or the other device closes it too.
fn prompt(node: Node, request: Uuid, pairing: Option<Fingerprint>, summary: String, body: String, accept_label: &'static str) {
    tokio::task::spawn_blocking(move || {
        let mut n = base(&summary, &body);
        n.action("accept", accept_label).action("decline", t!("Decline")).urgency(Urgency::Critical).timeout(Timeout::Never);
        match n.show() {
            Ok(handle) => {
                PROMPTS.lock().unwrap().get_or_insert_default().insert(request, (handle.id(), pairing));
                // Answered while it was appearing: nothing was there to close then
                if !node.is_pending(request) {
                    answered(request);
                }
                let forget = || PROMPTS.lock().unwrap().as_mut().map(|p| p.remove(&request));
                handle.wait_for_action(|action| match action {
                    "accept" => {
                        forget();
                        node.respond(request, true);
                    }
                    "decline" => {
                        forget();
                        node.respond(request, false);
                    }
                    // Clicking the body or dismissing leaves the decision to the app.
                    _ => {}
                })
            }
            Err(e) => debug!("notification failed: {e}"),
        }
    });
}

/// One notice per batch of imported photos, opening their folder.
fn imported(from: String, saved: Vec<PathBuf>) {
    if saved.is_empty() {
        return;
    }
    tokio::task::spawn_blocking(move || {
        let summary = if saved.len() == 1 { t!("Imported 1 photo from {device}", device = from) } else { t!("Imported {n} photos from {device}", n = saved.len(), device = from) };
        let folder = saved[0].parent().map(Path::to_path_buf).unwrap_or_default();
        let mut n = base(&summary, &folder.display().to_string());
        n.action("default", t!("Show Photos"));
        match n.show() {
            Ok(handle) => handle.wait_for_action(|action| {
                if action == "default" {
                    let _ = std::process::Command::new("xdg-open").arg(&folder).spawn();
                }
            }),
            Err(e) => debug!("notification failed: {e}"),
        }
    });
}

fn received(from: String, saved: Vec<PathBuf>) {
    tokio::task::spawn_blocking(move || {
        let (summary, target) = match saved.as_slice() {
            [one] => (t!("Received {name}", name = file_name(one)), one.clone()),
            many => (
                t!("Received {n} items", n = many.len()),
                many.first().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_default(),
            ),
        };
        let mut n = base(&summary, &t!("From {device}", device = from));
        n.action("default", t!("Open")).action("show", t!("Show in Folder"));
        match n.show() {
            Ok(handle) => handle.wait_for_action(|action| {
                let path = match action {
                    "default" => target.clone(),
                    "show" => target.parent().map(Path::to_path_buf).unwrap_or_default(),
                    _ => return,
                };
                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            }),
            Err(e) => debug!("notification failed: {e}"),
        }
    });
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn display_name(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

fn spaced(code: &str) -> String {
    if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code.to_string() }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", UNITS[unit]) } else { format!("{v:.0} {}", UNITS[unit]) }
}
