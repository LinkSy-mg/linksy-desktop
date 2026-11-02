//! The phones' conversations, made from their message notifications (WhatsApp, SMS,
//! Discord, mail…). Kept in memory only, like the notifications themselves.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use linksy_core::{DeviceInfo, Fingerprint, RemoteNotification};
use linksy_ipc::{Conversation, InboxMessage};
use tokio::sync::broadcast;

const MAX_CONVERSATIONS: usize = 100;
const MAX_MESSAGES: usize = 50;
const MAX_TEXT: usize = 4096;

const SMS: &[&str] = &["com.google.android.apps.messaging", "com.samsung.android.messaging", "com.android.mms", "com.android.messaging", "com.oneplus.mms"];
const MAIL: &[&str] = &[
    "com.google.android.gm",
    "com.microsoft.office.outlook",
    "com.samsung.android.email.provider",
    "net.thunderbird.android",
    "com.fsck.k9",
    "ch.protonmail.android",
    "com.yahoo.mobile.client.android.mail",
];
const SOCIAL: &[&str] = &["com.facebook.katana", "com.instagram.android", "com.twitter.android", "com.zhiliaoapp.musically", "com.linkedin.android"];
const CHAT: &[&str] = &[
    "com.whatsapp",
    "com.whatsapp.w4b",
    "org.telegram.messenger",
    "org.thunderdog.challegram",
    "org.thoughtcrime.securesms",
    "com.facebook.orca",
    "com.discord",
    "com.Slack",
    "com.microsoft.teams",
    "com.viber.voip",
    "jp.naver.line.android",
    "com.tencent.mm",
    "com.snapchat.android",
    "com.google.android.apps.dynamite",
    "im.vector.app",
];

#[derive(Clone)]
pub struct Inbox {
    inner: Arc<Mutex<Vec<Entry>>>,
    updates: broadcast::Sender<Conversation>,
    /// Where pictures and app icons are unpacked (the runtime folder, gone at logout).
    dir: PathBuf,
}

struct Entry {
    phone: Fingerprint,
    conversation: Conversation,
    messages: VecDeque<InboxMessage>,
    /// The phone's notification to answer through, while it has one.
    key: Option<String>,
}

/// What a notification says, as a message.
#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    kind: &'static str,
    title: String,
    sender: Option<String>,
    text: String,
    group: bool,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self { inner: Arc::default(), updates: broadcast::channel(64).0, dir }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Conversation> {
        self.updates.subscribe()
    }

    pub fn list(&self) -> Vec<Conversation> {
        let mut list: Vec<Conversation> = self.inner.lock().unwrap().iter().map(|e| e.conversation.clone()).collect();
        list.sort_by_key(|c| std::cmp::Reverse(c.at_ms));
        list
    }

    pub fn thread(&self, id: &str) -> Option<Vec<InboxMessage>> {
        self.inner.lock().unwrap().iter().find(|e| e.conversation.id == id).map(|e| e.messages.iter().cloned().collect())
    }

    /// The phone's notification for `id`, while it has one: the phone and its key.
    pub fn notification(&self, id: &str) -> Option<(Fingerprint, String, bool)> {
        let inbox = self.inner.lock().unwrap();
        let e = inbox.iter().find(|e| e.conversation.id == id)?;
        Some((e.phone, e.key.clone()?, e.conversation.can_reply))
    }

    /// A notification from `from`: a new message in a conversation, when it reads as one.
    /// Returns the code it carries, if any.
    pub fn posted(&self, from: &DeviceInfo, n: &RemoteNotification) -> Option<String> {
        let parsed = parse(n)?;
        let code = crate::codes::code_in(&format!("{} {}", n.title, n.body));
        let at_ms = n.time.unwrap_or_else(now_ms);
        let id = conversation_id(&from.id, &n.app, &parsed.title);
        let icon = n.icon.as_deref().and_then(|b64| unpack(&self.dir, &n.app, b64));
        let picture = n.picture.as_deref().and_then(|b64| unpack(&self.dir.join("people"), &id, b64));
        let mut inbox = self.inner.lock().unwrap();
        let i = match inbox.iter().position(|e| e.conversation.id == id) {
            Some(i) => i,
            None => {
                if inbox.len() >= MAX_CONVERSATIONS
                    && let Some(oldest) = inbox.iter().enumerate().min_by_key(|(_, e)| e.conversation.at_ms).map(|(i, _)| i)
                {
                    inbox.remove(oldest);
                }
                inbox.push(Entry {
                    phone: from.id,
                    conversation: Conversation {
                        id: id.clone(),
                        phone: from.id.to_string(),
                        phone_name: from.name.clone(),
                        app: n.app.clone(),
                        app_name: n.app_name.clone(),
                        kind: parsed.kind.into(),
                        title: parsed.title.clone(),
                        group: parsed.group,
                        icon: None,
                        picture: None,
                        last: None,
                        unread: 0,
                        can_reply: false,
                        at_ms,
                    },
                    messages: VecDeque::new(),
                    key: None,
                });
                inbox.len() - 1
            }
        };
        let e = &mut inbox[i];
        let message = InboxMessage { at_ms, sender: parsed.sender, text: parsed.text, code: code.clone(), mine: false };
        // A notification shown again, or updated with the same newest line, is no new message
        let fresh = !e.messages.back().is_some_and(|m| m.text == message.text && m.sender == message.sender);
        if fresh {
            e.messages.push_back(message.clone());
            if e.messages.len() > MAX_MESSAGES {
                e.messages.pop_front();
            }
            e.conversation.unread += 1;
            e.conversation.last = Some(message);
            e.conversation.at_ms = at_ms;
        }
        e.key = Some(n.key.clone());
        e.conversation.can_reply = n.can_reply;
        e.conversation.group |= parsed.group;
        e.conversation.icon = icon.or(e.conversation.icon.take());
        e.conversation.picture = picture.or(e.conversation.picture.take());
        let _ = self.updates.send(e.conversation.clone());
        code
    }

    /// The phone's notification `key` is gone (read, answered or swiped away there).
    pub fn removed(&self, from: &Fingerprint, key: &str) {
        let mut inbox = self.inner.lock().unwrap();
        for e in inbox.iter_mut().filter(|e| e.phone == *from && e.key.as_deref() == Some(key)) {
            e.key = None;
            e.conversation.can_reply = false;
            e.conversation.unread = 0;
            let _ = self.updates.send(e.conversation.clone());
        }
    }

    pub fn read(&self, id: &str) {
        let mut inbox = self.inner.lock().unwrap();
        if let Some(e) = inbox.iter_mut().find(|e| e.conversation.id == id && e.conversation.unread > 0) {
            e.conversation.unread = 0;
            let _ = self.updates.send(e.conversation.clone());
        }
    }

    /// `text` was sent from here.
    pub fn replied(&self, id: &str, text: &str) {
        let mut inbox = self.inner.lock().unwrap();
        if let Some(e) = inbox.iter_mut().find(|e| e.conversation.id == id) {
            let message = InboxMessage { at_ms: now_ms(), sender: None, text: text.into(), code: None, mine: true };
            e.messages.push_back(message.clone());
            if e.messages.len() > MAX_MESSAGES {
                e.messages.pop_front();
            }
            e.conversation.unread = 0;
            e.conversation.at_ms = message.at_ms;
            e.conversation.last = Some(message);
            let _ = self.updates.send(e.conversation.clone());
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn conversation_id(phone: &Fingerprint, app: &str, title: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (phone.to_string(), app, title.to_lowercase()).hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Write a picture or icon (base64 PNG or JPEG) where the apps and the notification server
/// can read it.
pub(crate) fn unpack(dir: &Path, name: &str, b64: &str) -> Option<PathBuf> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let ext = if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "jpg"
    } else {
        return None;
    };
    let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
    let path = dir.join(format!("{name}.{ext}"));
    if std::fs::read(&path).ok().as_deref() != Some(&bytes[..]) {
        std::fs::create_dir_all(dir).ok()?;
        std::fs::write(&path, &bytes).ok()?;
    }
    Some(path)
}

/// A notification as a message, when it is one: its kind, conversation, sender and text.
/// What the phone says comes first; for a phone that doesn't, the title is read the way
/// the common apps write it.
fn parse(n: &RemoteNotification) -> Option<Parsed> {
    let app = n.app.as_str();
    let kind = if SMS.contains(&app) {
        "sms"
    } else if MAIL.contains(&app) || n.category.as_deref() == Some("email") {
        "mail"
    } else if SOCIAL.contains(&app) || n.category.as_deref() == Some("social") {
        "social"
    } else if CHAT.contains(&app) || n.category.as_deref() == Some("msg") || n.can_reply {
        "chat"
    } else {
        return None;
    };
    let body = n.body.trim();
    // "12 messages from 3 chats": the app's summary, not a message
    if n.title.trim() == n.app_name || body.is_empty() || is_summary(body) {
        return None;
    }
    let mut title = n.conversation.clone().unwrap_or_else(|| clean_title(&n.title));
    let mut sender = n.sender.clone();
    let mut group = n.group;
    let mut text = body.to_string();
    // "Bob @ Family"
    if sender.is_none()
        && n.conversation.is_none()
        && let Some((who, chat)) = n.title.split_once(" @ ")
    {
        sender = Some(who.trim().into());
        title = clean_title(chat);
        group = true;
    }
    // "Bob: Photos from Sunday are up", in a group
    if kind == "chat"
        && let Some((who, rest)) = body.split_once(": ")
        && looks_like_name(who)
        && Some(who) != Some(title.as_str())
        && (sender.is_none() || sender.as_deref() == Some(who))
    {
        sender = Some(who.into());
        text = rest.into();
        group = true;
    }
    if kind == "mail" {
        // The sender is the conversation; the subject and the start of the mail are the text
        sender = None;
    }
    text.truncate(text.char_indices().nth(MAX_TEXT).map_or(text.len(), |(i, _)| i));
    Some(Parsed { kind, title, sender, text, group })
}

fn is_summary(body: &str) -> bool {
    let first = body.split_whitespace().next().unwrap_or("");
    first.chars().all(|c| c.is_ascii_digit()) && !first.is_empty() && body.contains("message") && (body.contains("chat") || body.contains("conversation"))
}

/// "Carol (2 messages)" or "Carol (3)" is Carol.
fn clean_title(title: &str) -> String {
    let t = title.trim();
    match t.rsplit_once(" (") {
        Some((name, rest)) if rest.ends_with(')') && rest.chars().next().is_some_and(|c| c.is_ascii_digit()) => name.trim().into(),
        _ => t.into(),
    }
}

fn looks_like_name(s: &str) -> bool {
    let s = s.trim();
    (1..=40).contains(&s.chars().count()) && !s.contains(['\n', '/']) && s.split_whitespace().count() <= 4 && !s.starts_with("http")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(app: &str, app_name: &str, title: &str, body: &str) -> RemoteNotification {
        RemoteNotification { key: "k".into(), app: app.into(), app_name: app_name.into(), title: title.into(), body: body.into(), can_reply: true, ..Default::default() }
    }

    #[test]
    fn reads_the_common_apps() {
        let p = parse(&n("com.whatsapp", "WhatsApp", "Alice", "Lunch?")).unwrap();
        assert_eq!((p.kind, p.title.as_str(), p.sender, p.text.as_str(), p.group), ("chat", "Alice", None, "Lunch?", false));
        let p = parse(&n("com.whatsapp", "WhatsApp", "Family", "Bob: Photos are up")).unwrap();
        assert_eq!((p.title.as_str(), p.sender.as_deref(), p.text.as_str(), p.group), ("Family", Some("Bob"), "Photos are up", true));
        let p = parse(&n("com.whatsapp", "WhatsApp", "Bob @ Family", "Photos are up")).unwrap();
        assert_eq!((p.title.as_str(), p.sender.as_deref(), p.group), ("Family", Some("Bob"), true));
        let p = parse(&n("com.facebook.orca", "Messenger", "Carol (2 messages)", "See you tomorrow!")).unwrap();
        assert_eq!(p.title, "Carol");
        let p = parse(&n("com.google.android.gm", "Gmail", "GitHub", "New issue: taps")).unwrap();
        assert_eq!((p.kind, p.title.as_str(), p.sender, p.text.as_str()), ("mail", "GitHub", None, "New issue: taps"));
        assert_eq!(parse(&n("com.google.android.apps.messaging", "Messages", "BNI", "Code 4829")).unwrap().kind, "sms");
    }

    #[test]
    fn what_the_phone_says_wins() {
        let x = RemoteNotification { conversation: Some("#general".into()), sender: Some("ana".into()), group: true, ..n("com.discord", "Discord", "Linksy · #general", "ana: hi") };
        let p = parse(&x).unwrap();
        assert_eq!((p.title.as_str(), p.sender.as_deref(), p.text.as_str()), ("#general", Some("ana"), "hi"));
    }

    #[test]
    fn leaves_what_isnt_a_message() {
        assert!(parse(&n("com.whatsapp", "WhatsApp", "WhatsApp", "12 messages from 3 chats")).is_none());
        assert!(parse(&RemoteNotification { can_reply: false, ..n("com.spotify.music", "Spotify", "Now playing", "Song") }).is_none());
        assert!(parse(&n("com.whatsapp", "WhatsApp", "Alice", "")).is_none());
    }

    #[test]
    fn one_conversation_per_chat_and_no_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().into());
        let phone = DeviceInfo { id: Fingerprint::of(b"p"), name: "Pixel".into(), kind: linksy_core::DeviceKind::Phone, os: "android".into(), features: vec![], away: None };
        let mut a = n("com.whatsapp", "WhatsApp", "Alice", "Lunch?");
        inbox.posted(&phone, &a);
        inbox.posted(&phone, &a);
        a.body = "At noon?".into();
        inbox.posted(&phone, &a);
        let code = inbox.posted(&phone, &n("com.google.android.apps.messaging", "Messages", "BNI", "Your code is 482913"));
        assert_eq!(code.as_deref(), Some("482913"));
        let list = inbox.list();
        assert_eq!(list.len(), 2);
        let alice = list.iter().find(|c| c.title == "Alice").unwrap();
        assert_eq!((alice.unread, alice.last.as_ref().unwrap().text.as_str()), (2, "At noon?"));
        assert_eq!(inbox.thread(&alice.id).unwrap().len(), 2);
        inbox.removed(&phone.id, "k");
        assert!(inbox.list().iter().all(|c| !c.can_reply && c.unread == 0));
    }
}
