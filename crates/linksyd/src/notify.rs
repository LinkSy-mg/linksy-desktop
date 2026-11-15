//! Native desktop notifications for things that need the user, even with no window open.
//! Consent prompts carry Accept/Decline buttons; whichever answer comes first (here or in
//! the app) wins, and the prompt goes away either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use linksy_core::{CallState, Direction, Event, Fingerprint, Node, SessionKind, Uuid};
use linksy_i18n::t;
use notify_rust::{Hint, Notification, Timeout, Urgency};
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

const APP: &str = "Linksy";
pub(crate) const DESKTOP_ENTRY: &str = "mg.diary.linksy.Linksy";
/// Battery level, in percent, at which a device's low battery is announced (once).
const LOW_BATTERY: u8 = 15;
/// A phone that leaves its touchpad page and comes back starts a new session each time: the
/// notice that a session was allowed without asking is not repeated within this.
const SAID_FOR: Duration = Duration::from_secs(60);

/// What a prompt's "always" button allows a device from now on.
#[derive(Clone, Copy)]
enum Always {
    Files(Fingerprint),
    Screen(Fingerprint),
}

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
    // Session requests asked about: one that starts without being asked is said.
    let mut asked: std::collections::HashSet<Uuid> = Default::default();
    // When a device's allowed-without-asking session was last said
    let mut said: HashMap<(Fingerprint, String), Instant> = Default::default();
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
        match event {
            Event::PairingRequest { request, device, code, .. } => {
                let body = t!("Check that {device} shows the code {code}.", device = device.name, code = spaced(&code));
                prompt(node.clone(), request, Some(device.id), None, t!("Pair with {device}?", device = device.name), body, t!("Pair"));
            }
            Event::TransferRequest { transfer, from, files, total, .. } => {
                let what = match files.as_slice() {
                    [one] => display_name(&one.path),
                    many => t!("{n} items", n = many.len()),
                };
                let body = format!("{what} · {}", human_size(total));
                prompt(node.clone(), transfer, None, Some(Always::Files(from.id)), t!("{device} wants to send you files", device = from.name), body, t!("Accept"));
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
                    // Never asked of a computer: the core answers that this one can't
                    SessionKind::Sound => continue,
                    SessionKind::Terminal => (
                        t!("{device} wants a terminal on this computer", device = from.name),
                        t!("The right code was typed on {device}. Its terminal runs commands as you until it's ended. Linksy asks once a day.", device = from.name),
                    ),
                };
                asked.insert(request);
                let always = kind.may_always_allow().then_some(Always::Screen(from.id));
                prompt(node.clone(), request, None, always, summary, body, t!("Allow"));
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
            Event::SessionStarted { session, peer, kind, sharing } => {
                answered(session);
                // Allowed for good: still say it, the moment it starts
                if !asked.remove(&session) && sharing && kind.may_always_allow() {
                    let summary = match kind {
                        SessionKind::Control => t!("{name} is controlling your screen", name = peer.name),
                        SessionKind::Input => t!("{name} is your touchpad and keyboard", name = peer.name),
                        _ => t!("{name} is watching your screen", name = peer.name),
                    };
                    if !already_said(&mut said, (peer.id, summary.clone()), Instant::now()) {
                        show(summary, t!("Allowed without asking, as you chose. Stop it in Linksy, or turn it off on {device}'s page.", device = peer.name));
                    }
                }
            }
            Event::SessionEnded { session } => {
                asked.remove(&session);
                answered(session);
            }
            Event::PairingFailed { device, .. } => pairing_over(device.id),
            Event::Paired { device } => {
                pairing_over(device.id);
                show(t!("Paired with {device}", device = device.name), t!("You can now share files and more.").into());
            }
            Event::Notification { from, title, body } => {
                let body = with_sender(&body, &from.name, t!("From {device}", device = from.name));
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

/// Whether this notice was already shown within `SAID_FOR`; notes it when it wasn't.
fn already_said(said: &mut HashMap<(Fingerprint, String), Instant>, notice: (Fingerprint, String), now: Instant) -> bool {
    said.retain(|_, at| now.saturating_duration_since(*at) < SAID_FOR);
    if said.contains_key(&notice) {
        return true;
    }
    said.insert(notice, now);
    false
}

/// A phone's notification with the device it came from added under the text, unless the
/// text already says it (a Ping's does, which showed the line twice). The phone's text is
/// cut first, to leave room, so the line saying who it came from is never the part lost.
fn with_sender(body: &str, device: &str, from: String) -> String {
    let from = clean(&from, LABEL_MAX, false);
    if body.is_empty() {
        return from;
    }
    let room = BODY_MAX.saturating_sub(from.chars().count() + 1);
    let body = clean(body, room, true);
    if body.contains(device) {
        body
    } else {
        format!("{body}\n{from}")
    }
}

const SUMMARY_MAX: usize = 200;
const BODY_MAX: usize = 2000;
const LABEL_MAX: usize = 80;

/// Characters that draw nothing or reorder the text around them, so a phone could use them
/// to hide or disguise words. The joiners (U+200C, U+200D) stay: Persian, Indic scripts
/// and emoji sequences need them.
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{200b}' | '\u{200e}' | '\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2066}'..='\u{2069}'
        | '\u{2028}' | '\u{2029}'
        | '\u{061c}' | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}'
        | '\u{e0000}'..='\u{e007f}')
}

/// Text from another device made safe to show: control and invisible characters dropped
/// (a body keeps its line breaks) and cut to `max` characters with an ellipsis. It is
/// plain text still: `escape_markup` is for servers that render markup.
fn clean(text: &str, max: usize, keep_lines: bool) -> String {
    let mut kept: Vec<char> = text
        .chars()
        .map(|c| if c == '\t' || (c == '\n' && !keep_lines) { ' ' } else { c })
        .filter(|c| *c == '\n' || !(c.is_control() || is_invisible(*c)))
        .take(max + 1)
        .collect();
    if kept.len() > max {
        kept.truncate(max.saturating_sub(1));
        kept.push('…');
    }
    kept.into_iter().collect()
}

/// `& < >` escaped, so a server that renders body markup shows a phone's text as text, with
/// no links or formatting of the phone's choosing.
fn escape_markup(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
        out
    })
}

/// A phone's label (a button, an app's name) as plain text, short enough for a button.
pub(crate) fn clean_label(text: &str) -> String {
    clean(text, LABEL_MAX, false)
}

fn clean_summary(text: &str) -> String {
    clean(text, SUMMARY_MAX, false)
}

/// The body as the server will show it: escaped only when it renders markup, because one
/// that doesn't (dunst with markup=no) would show `&amp;` as typed.
fn clean_body(text: &str, markup: bool) -> String {
    let body = clean(text, BODY_MAX, true);
    if markup {
        escape_markup(&body)
    } else {
        body
    }
}

/// Whether the notification server renders body markup. Asked once; a server that can't be
/// asked is treated as not rendering it, and asked again next time.
fn body_markup() -> bool {
    static MARKUP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if let Some(known) = MARKUP.get() {
        return *known;
    }
    match notify_rust::get_capabilities() {
        Ok(caps) => *MARKUP.get_or_init(|| caps.iter().any(|c| c == "body-markup")),
        Err(e) => {
            debug!("could not ask the notification server what it supports: {e}");
            false
        }
    }
}

pub fn base(summary: &str, body: &str) -> Notification {
    // The summary is never markup, so it is never escaped
    let (summary, body) = (clean_summary(summary), clean_body(body, body_markup()));
    let mut n = Notification::new();
    n.appname(APP).summary(&summary).body(&body).icon(DESKTOP_ENTRY).hint(Hint::DesktopEntry(DESKTOP_ENTRY.into()));
    n
}

pub(crate) fn show(summary: String, body: String) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = base(&summary, &body).show() {
            debug!("notification failed: {e}");
        }
    });
}

/// A phone's saved deck uses actions that are off here (docs/protocol.md §4.26): which, and
/// an Allow that turns them on for that phone. Nothing else decides it: the phone can't.
pub(crate) fn deck_asks(phone: Fingerprint, device: String, actions: Vec<(String, String)>) {
    tokio::task::spawn_blocking(move || {
        let names = actions.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>().join(", ");
        let mut n = base(&t!("{device}'s deck needs your OK", device = device), &t!("It uses {actions}, which are off on this computer. Allow them here, or choose in Linksy, on {device}'s page.", actions = names, device = device));
        n.action("allow", t!("Allow"));
        match n.show() {
            Ok(handle) => handle.wait_for_action(|action| {
                if action != "allow" {
                    return;
                }
                let Some(deck) = crate::deck::DECK.get() else { return };
                for (id, _) in &actions {
                    if let Err(e) = deck.set_action(&phone, id, true) {
                        debug!("could not allow {id}: {e}");
                    }
                }
            }),
            Err(e) => debug!("notification failed: {e}"),
        }
    });
}

/// A notification with Accept/Decline that answers the request when clicked. It stays until
/// answered (critical, no timeout), and goes when it is: clicking a button closes it (it
/// isn't resident), and an answer from the app or the other device closes it too.
/// `always`: what the prompt can also allow from now on.
fn prompt(node: Node, request: Uuid, pairing: Option<Fingerprint>, always: Option<Always>, summary: String, body: String, accept_label: &'static str) {
    tokio::task::spawn_blocking(move || {
        let mut n = base(&summary, &body);
        n.action("accept", accept_label);
        match always {
            Some(Always::Files(_)) => n.action("always", t!("Always Accept")),
            Some(Always::Screen(_)) => n.action("always", t!("Always Allow")),
            None => &mut n,
        };
        n.action("decline", t!("Decline")).urgency(Urgency::Critical).timeout(Timeout::Never);
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
                    "always" => {
                        forget();
                        let set = match always {
                            Some(Always::Files(id)) => node.set_auto_accept(&id, true),
                            Some(Always::Screen(id)) => node.set_allow_screen(&id, true),
                            None => Ok(()),
                        };
                        if let Err(e) = set {
                            debug!("always failed: {e}");
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_allowed_without_asking_is_said_once_a_minute() {
        let (a, b) = (Fingerprint([1; 32]), Fingerprint([2; 32]));
        let mut said = HashMap::new();
        let start = Instant::now();
        let touchpad = |who| (who, "Pixel is your touchpad and keyboard".to_string());
        assert!(!already_said(&mut said, touchpad(a), start));
        // The phone comes back a few seconds later
        assert!(already_said(&mut said, touchpad(a), start + Duration::from_secs(5)));
        // Another device, or another kind of session, is its own notice
        assert!(!already_said(&mut said, touchpad(b), start + Duration::from_secs(6)));
        assert!(!already_said(&mut said, (a, "Pixel is controlling your screen".into()), start + Duration::from_secs(7)));
        // After a minute it is said again
        assert!(!already_said(&mut said, touchpad(a), start + SAID_FOR + Duration::from_secs(1)));
    }

    #[test]
    fn a_notification_names_its_phone_once() {
        let from = || "From Pixel".to_string();
        assert_eq!(with_sender("", "Pixel", from()), "From Pixel");
        assert_eq!(with_sender("Lunch?", "Pixel", from()), "Lunch?\nFrom Pixel");
        assert_eq!(with_sender("From Pixel", "Pixel", from()), "From Pixel");
        assert_eq!(with_sender("Ping from Pixel", "Pixel", from()), "Ping from Pixel");
    }

    #[test]
    fn markup_is_escaped_only_where_the_server_renders_it() {
        let text = "<b>hi</b> & <a href=\"http://x\">y</a>";
        assert_eq!(clean_body(text, true), "&lt;b&gt;hi&lt;/b&gt; &amp; &lt;a href=\"http://x\"&gt;y&lt;/a&gt;");
        assert_eq!(clean_body("Tom & Jerry", false), "Tom & Jerry");
        // The summary is plain text on every server
        assert_eq!(clean_summary("Tom & Jerry <3"), "Tom & Jerry <3");
    }

    #[test]
    fn peer_text_loses_control_characters() {
        assert_eq!(clean_summary("a\u{1b}[31mb\nc\u{0}\u{7f}d\te"), "a[31mb cd e");
        // A body keeps its line breaks
        assert_eq!(clean_body("one\r\ntwo\u{0}\nthree\u{202e}", false), "one\ntwo\nthree");
    }

    #[test]
    fn invisible_characters_go_but_joiners_stay() {
        for c in ['\u{200b}', '\u{200e}', '\u{202e}', '\u{2066}', '\u{2028}', '\u{2029}', '\u{61c}', '\u{feff}', '\u{fff9}', '\u{e0041}'] {
            assert!(is_invisible(c), "{c:?}");
        }
        for c in ['\u{200c}', '\u{200d}', 'a', 'é', '😀'] {
            assert!(!is_invisible(c), "{c:?}");
        }
        assert_eq!(clean_summary("a\u{200b}b\u{e0041}c"), "abc");
        // A Persian word and an emoji sequence keep their joiners
        assert_eq!(clean_summary("می\u{200c}خواهم"), "می\u{200c}خواهم");
        assert_eq!(clean_summary("👩\u{200d}💻"), "👩\u{200d}💻");
    }

    #[test]
    fn peer_text_is_capped_on_character_boundaries() {
        let long = "é".repeat(5000);
        let s = clean_summary(&long);
        assert_eq!(s.chars().count(), SUMMARY_MAX);
        assert!(s.ends_with('…'));
        let b = clean_body(&"😀".repeat(BODY_MAX * 2), false);
        assert_eq!(b.chars().count(), BODY_MAX);
        assert!(b.ends_with('…'));
        // Text at the limit is left alone, and a cut never splits an escape
        assert_eq!(clean_summary(&"x".repeat(SUMMARY_MAX)), "x".repeat(SUMMARY_MAX));
        let amp = clean_body(&"&".repeat(BODY_MAX + 10), true);
        assert!(amp.trim_end_matches('…').ends_with("&amp;"));
        // A megabyte is cheap to clean
        assert_eq!(clean_body(&"a".repeat(1 << 20), false).chars().count(), BODY_MAX);
        assert_eq!(clean_label(&"l".repeat(500)).chars().count(), LABEL_MAX);
    }

    #[test]
    fn a_long_notification_still_says_who_it_is_from() {
        let from = || "From Pixel".to_string();
        let body = with_sender(&"x".repeat(1 << 16), "Pixel", from());
        assert!(body.ends_with("\nFrom Pixel"), "{body:?}");
        assert!(body.chars().count() <= BODY_MAX);
        // base cuts nothing more, so the sender survives it too
        assert_eq!(clean_body(&body, false), body);
    }

    #[test]
    fn base_leaves_the_summary_unescaped() {
        let n = base("<i>Jenna</i>", "see <a href=\"x\">this</a>");
        assert_eq!(n.summary, "<i>Jenna</i>");
    }
}
