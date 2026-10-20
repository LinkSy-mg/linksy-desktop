//! Native desktop notifications for things that need the user, even with no window open.
//! Consent prompts carry Accept/Decline buttons; whichever answer comes first (here or in
//! the app) wins.

use std::path::{Path, PathBuf};

use linksy_core::{Direction, Event, Fingerprint, Node, SessionKind, Uuid};
use notify_rust::{Hint, Notification, Timeout, Urgency};
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

const APP: &str = "Linksy";
const DESKTOP_ENTRY: &str = "mg.diary.linksy.Linksy";
/// Battery level, in percent, at which a device's low battery is announced (once).
const LOW_BATTERY: u8 = 15;

pub async fn run(node: Node) {
    let mut events = node.subscribe();
    // Incoming transfers: remember who sent what so completion can say it.
    let mut incoming: std::collections::HashMap<Uuid, String> = Default::default();
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
                let body = format!("Check that {} shows the code {}.", device.name, spaced(&code));
                prompt(node.clone(), request, format!("Pair with {}?", device.name), body, "Pair");
            }
            Event::TransferRequest { transfer, from, files, total } => {
                let what = match files.as_slice() {
                    [one] => display_name(&one.path),
                    many => format!("{} items", many.len()),
                };
                let body = format!("{what} · {}", human_size(total));
                prompt(node.clone(), transfer, format!("{} wants to send you files", from.name), body, "Accept");
            }
            Event::SessionRequest { request, from, kind } => {
                let (summary, body) = match kind {
                    SessionKind::Control => (
                        format!("{} wants to control your screen", from.name),
                        format!("{} will see your screen and use your mouse and keyboard until you stop.", from.name),
                    ),
                    SessionKind::Camera => (
                        format!("{} wants to use your camera", from.name),
                        format!("Your camera will be on and {} will see it until you stop.", from.name),
                    ),
                    SessionKind::Display => (
                        format!("{} wants to use this screen as a second display", from.name),
                        format!("Linksy will fill this screen with {}'s second display, and your mouse and keyboard here will work on it.", from.name),
                    ),
                    SessionKind::Screen => (
                        format!("{} wants to see your screen", from.name),
                        format!("{} will see everything on your screen until you stop sharing.", from.name),
                    ),
                };
                prompt(node.clone(), request, summary, body, "Allow");
            }
            Event::TransferStarted { transfer, peer, direction: Direction::Incoming, .. } => {
                incoming.insert(transfer, peer.name);
            }
            Event::TransferComplete { transfer, saved } => {
                if let Some(from) = incoming.remove(&transfer) {
                    received(from, saved);
                }
            }
            Event::TransferFailed { transfer, reason } => {
                if let Some(from) = incoming.remove(&transfer)
                    && reason != "cancelled"
                {
                    show(format!("Transfer from {from} failed"), capitalize(&reason));
                }
            }
            Event::Paired { device } => show(format!("Paired with {}", device.name), "You can now share files and more.".into()),
            Event::Notification { from, title, body } => {
                let body = if body.is_empty() { format!("From {}", from.name) } else { format!("{body}\nFrom {}", from.name) };
                show(title, body);
            }
            Event::Battery { from, battery } if battery.level <= LOW_BATTERY && !battery.charging => {
                if warned.insert(from.id) {
                    show(format!("{} is low on battery", from.name), format!("{}% left", battery.level));
                }
            }
            Event::Battery { from, battery } if battery.charging || battery.level > LOW_BATTERY + 5 => {
                warned.remove(&from.id);
            }
            Event::Ring { from, on: true } => {
                tokio::task::spawn_blocking(move || {
                    let mut n = base(&format!("{} is looking for this computer", from.name), "Here it is!");
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

fn show(summary: String, body: String) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = base(&summary, &body).show() {
            debug!("notification failed: {e}");
        }
    });
}

/// A notification with Accept/Decline that answers the request when clicked.
fn prompt(node: Node, request: Uuid, summary: String, body: String, accept_label: &'static str) {
    tokio::task::spawn_blocking(move || {
        let mut n = base(&summary, &body);
        n.action("accept", accept_label)
            .action("decline", "Decline")
            .urgency(Urgency::Critical)
            .hint(Hint::Resident(true))
            .timeout(Timeout::Never);
        match n.show() {
            Ok(handle) => handle.wait_for_action(|action| match action {
                "accept" => {
                    node.respond(request, true);
                }
                "decline" => {
                    node.respond(request, false);
                }
                // Clicking the body or dismissing leaves the decision to the app.
                _ => {}
            }),
            Err(e) => debug!("notification failed: {e}"),
        }
    });
}

fn received(from: String, saved: Vec<PathBuf>) {
    tokio::task::spawn_blocking(move || {
        let (summary, target) = match saved.as_slice() {
            [one] => (format!("Received {}", file_name(one)), one.clone()),
            many => (
                format!("Received {} items", many.len()),
                many.first().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_default(),
            ),
        };
        let mut n = base(&summary, &format!("From {from}"));
        n.action("default", "Open").action("show", "Show in Folder");
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
