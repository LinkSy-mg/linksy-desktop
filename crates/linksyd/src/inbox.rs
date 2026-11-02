//! The phones' conversations, made from their message notifications (WhatsApp, SMS,
//! Discord, mail…). Kept in memory only, like the notifications themselves.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use linksy_core::{ConversationMessage, DeviceInfo, Fingerprint, RemoteNotification};
use linksy_ipc::{Alert, Conversation, InboxMessage};
use tokio::sync::broadcast;

const MAX_CONVERSATIONS: usize = 100;
const MAX_MESSAGES: usize = 200;
const MAX_ALERTS: usize = 30;
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
    /// The other notifications, newest first.
    alerts: Arc<Mutex<VecDeque<Alert>>>,
    updates: broadcast::Sender<Conversation>,
    /// Where pictures and app icons are unpacked (the runtime folder, gone at logout).
    dir: PathBuf,
}

struct Entry {
    phone: Fingerprint,
    conversation: Conversation,
    /// Oldest first.
    messages: VecDeque<InboxMessage>,
    /// The phone's notification to answer through, while it has one.
    key: Option<String>,
    /// Its notification's key, kept after it's gone.
    seen_key: String,
    /// The conversation as the phone names it, to ask for its history.
    asked: String,
}

/// What a notification says, as a message.
#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    kind: &'static str,
    title: String,
    sender: Option<String>,
    text: String,
    group: bool,
    /// Written by the phone's owner.
    mine: bool,
    /// The chat came named after its owner ("Vous"): a one-to-one chat they just answered.
    titled_me: bool,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self { inner: Arc::default(), alerts: Arc::default(), updates: broadcast::channel(64).0, dir }
    }

    pub fn alerts(&self) -> Vec<Alert> {
        self.alerts.lock().unwrap().iter().cloned().collect()
    }

    /// The keys of `phone`'s other notifications.
    pub fn alert_keys(&self, phone: &Fingerprint) -> Vec<String> {
        let phone = phone.to_string();
        self.alerts.lock().unwrap().iter().filter(|a| a.phone == phone).map(|a| a.key.clone()).collect()
    }

    /// A notification that isn't a message. Returns the code it carries, if any.
    fn alert(&self, from: &DeviceInfo, n: &RemoteNotification) -> Option<String> {
        if kind(n).is_some() || (n.title.trim().is_empty() && n.body.trim().is_empty()) {
            return None;
        }
        let code = crate::codes::code_in(&format!("{} {}", n.title, n.body));
        let mut body = n.body.trim().to_string();
        body.truncate(body.char_indices().nth(MAX_TEXT).map_or(body.len(), |(i, _)| i));
        let alert = Alert {
            phone: from.id.to_string(),
            key: n.key.clone(),
            app: n.app.clone(),
            app_name: n.app_name.clone(),
            title: n.title.trim().chars().take(200).collect(),
            body,
            icon: n.icon.as_deref().and_then(|b64| unpack(&self.dir, &n.app, b64)),
            code: code.clone(),
            at_ms: n.time.unwrap_or_else(now_ms),
        };
        let mut alerts = self.alerts.lock().unwrap();
        alerts.retain(|a| !(a.phone == alert.phone && a.key == alert.key));
        alerts.push_front(alert);
        alerts.truncate(MAX_ALERTS);
        code
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
        let Some(mut parsed) = parse(n) else { return self.alert(from, n) };
        let code = crate::codes::code_in(&format!("{} {}", n.title, n.body));
        let at_ms = n.time.unwrap_or_else(now_ms);
        let mut inbox = self.inner.lock().unwrap();
        // Named after the owner: the chat this notification was for until now
        if parsed.titled_me
            && let Some(e) = inbox.iter().filter(|e| e.phone == from.id && e.seen_key == n.key).max_by_key(|e| e.conversation.at_ms)
        {
            parsed.title = e.conversation.title.clone();
        }
        let id = conversation_id(&from.id, &n.app, &parsed.title);
        let icon = n.icon.as_deref().and_then(|b64| unpack(&self.dir, &n.app, b64));
        // The owner's picture isn't the chat's
        let picture = n.picture.as_deref().filter(|_| !parsed.mine).and_then(|b64| unpack(&self.dir.join("people"), &id, b64));
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
                        older: false,
                    },
                    messages: VecDeque::new(),
                    key: None,
                    seen_key: String::new(),
                    asked: parsed.title.clone(),
                });
                inbox.len() - 1
            }
        };
        let e = &mut inbox[i];
        let message = InboxMessage { at_ms, sender: parsed.sender, text: parsed.text, code: code.clone(), mine: parsed.mine };
        // A notification shown again, or updated with the same newest line (or the reply
        // sent from here), is no new message
        let fresh = !e.messages.back().is_some_and(|m| m.text == message.text && m.sender == message.sender && m.mine == message.mine);
        if fresh {
            e.messages.push_back(message.clone());
            if message.mine {
                e.conversation.unread = 0;
            } else {
                e.conversation.unread += 1;
            }
            e.conversation.last = Some(message);
            e.conversation.at_ms = at_ms;
        }
        // The earlier messages the notification carries fill in what was missed.
        for m in &n.messages {
            add_message(&mut e.messages, from_phone(m, at_ms));
        }
        while e.messages.len() > MAX_MESSAGES {
            e.messages.pop_front();
        }
        if !parsed.titled_me {
            e.asked = n.conversation.clone().unwrap_or_else(|| parsed.title.clone());
        }
        e.conversation.older |= from.features.iter().any(|f| f == "inbox_history");
        e.key = Some(n.key.clone());
        e.seen_key = n.key.clone();
        e.conversation.can_reply = n.can_reply;
        e.conversation.group |= parsed.group;
        e.conversation.icon = icon.or(e.conversation.icon.take());
        e.conversation.picture = picture.or(e.conversation.picture.take());
        let _ = self.updates.send(e.conversation.clone());
        code
    }

    /// The phone's notification `key` is gone (read, answered or swiped away there); with
    /// `reply_kept`, the phone still answers its conversation through it.
    pub fn removed(&self, from: &Fingerprint, key: &str, reply_kept: bool) {
        let phone = from.to_string();
        self.alerts.lock().unwrap().retain(|a| !(a.phone == phone && a.key == key));
        let mut inbox = self.inner.lock().unwrap();
        for e in inbox.iter_mut().filter(|e| e.phone == *from && e.key.as_deref() == Some(key)) {
            if !reply_kept {
                e.key = None;
                e.conversation.can_reply = false;
            }
            e.conversation.unread = 0;
            let _ = self.updates.send(e.conversation.clone());
        }
    }

    /// Where to ask for conversation `id`'s earlier messages: the phone, the app, the
    /// conversation as the phone names it, and the time of the oldest one here.
    pub fn older_than(&self, id: &str) -> Option<(Fingerprint, String, String, u64)> {
        let inbox = self.inner.lock().unwrap();
        let e = inbox.iter().find(|e| e.conversation.id == id && e.conversation.older)?;
        let oldest = e.messages.front().map_or(e.conversation.at_ms, |m| m.at_ms);
        Some((e.phone, e.conversation.app.clone(), e.asked.clone(), oldest))
    }

    /// Earlier messages from the phone; `more`: it has older ones still.
    pub fn add_older(&self, phone: &Fingerprint, app: &str, conversation: &str, messages: &[ConversationMessage], more: bool) {
        let mut inbox = self.inner.lock().unwrap();
        let Some(e) = inbox.iter_mut().find(|e| e.phone == *phone && e.conversation.app == app && e.asked == conversation) else { return };
        let before = e.messages.front().map_or(e.conversation.at_ms, |m| m.at_ms);
        for m in messages {
            add_message(&mut e.messages, from_phone(m, before));
        }
        // The newest stay; older ones beyond what's kept are asked again if wanted.
        while e.messages.len() > MAX_MESSAGES {
            e.messages.pop_front();
        }
        e.conversation.older = more;
        let _ = self.updates.send(e.conversation.clone());
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
            while e.messages.len() > MAX_MESSAGES {
                e.messages.pop_front();
            }
            e.conversation.unread = 0;
            e.conversation.at_ms = message.at_ms;
            e.conversation.last = Some(message);
            let _ = self.updates.send(e.conversation.clone());
        }
    }
}

/// A message the phone kept, with its code; one without a time goes just before `before`.
fn from_phone(m: &ConversationMessage, before: u64) -> (InboxMessage, bool) {
    let mut text = m.text.trim().to_string();
    text.truncate(text.char_indices().nth(MAX_TEXT).map_or(text.len(), |(i, _)| i));
    let mine = m.mine || m.sender.as_deref().is_some_and(is_me);
    let message = InboxMessage { at_ms: m.time.unwrap_or(before.saturating_sub(1)), sender: m.sender.clone().filter(|_| !mine), code: crate::codes::code_in(&text), text, mine };
    (message, m.time.is_some())
}

/// Put `m` in its place by time, unless it's there already: same words from the same
/// side, and at the same time when it has one ("ok" twice is two messages).
fn add_message(messages: &mut VecDeque<InboxMessage>, (m, timed): (InboxMessage, bool)) {
    let same = |o: &InboxMessage| {
        o.text == m.text && o.mine == m.mine && (o.sender == m.sender || o.sender.is_none() || m.sender.is_none()) && (!timed || o.at_ms.abs_diff(m.at_ms) <= 1000)
    };
    if m.text.is_empty() || messages.iter().any(same) {
        return;
    }
    let at = messages.iter().position(|o| o.at_ms > m.at_ms).unwrap_or(messages.len());
    messages.insert(at, m);
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
    let kind = kind(n)?;
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
        if let Some(subject) = n.subject.as_deref().map(str::trim).filter(|s| !s.is_empty() && !text.starts_with(s)) {
            text = format!("{subject}\n{text}");
        }
    }
    text.truncate(text.char_indices().nth(MAX_TEXT).map_or(text.len(), |(i, _)| i));
    let titled_me = is_me(&title);
    let mine = n.mine || titled_me || sender.as_deref().is_some_and(is_me);
    if mine {
        sender = None;
    }
    if titled_me && !is_me(&n.title) && !n.title.trim().is_empty() {
        title = clean_title(&n.title);
    }
    Some(Parsed { kind, title, sender, text, group, mine, titled_me })
}

/// How chats name the phone's owner in their notifications, in the common languages.
fn is_me(name: &str) -> bool {
    const ME: &[&str] = &[
        "you", "me", "vous", "toi", "moi", "tú", "tu", "yo", "usted", "você", "eu", "du", "ich", "io", "jij", "ik", "вы", "ты", "я", "你", "我",
        "あなた", "自分", "ianao", "izaho",
    ];
    let name = name.trim().to_lowercase();
    ME.contains(&name.as_str())
}

/// Which kind of messaging app it comes from, if it's one.
fn kind(n: &RemoteNotification) -> Option<&'static str> {
    let app = n.app.as_str();
    if SMS.contains(&app) {
        Some("sms")
    } else if MAIL.contains(&app) || n.category.as_deref() == Some("email") {
        Some("mail")
    } else if SOCIAL.contains(&app) || n.category.as_deref() == Some("social") {
        Some("social")
    } else if CHAT.contains(&app) || n.category.as_deref() == Some("msg") || n.can_reply {
        Some("chat")
    } else {
        None
    }
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
        inbox.removed(&phone.id, "k", false);
        assert!(inbox.list().iter().all(|c| !c.can_reply && c.unread == 0));
    }

    #[test]
    fn the_owners_messages_are_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().into());
        let phone = DeviceInfo { id: Fingerprint::of(b"p"), name: "Pixel".into(), kind: linksy_core::DeviceKind::Phone, os: "android".into(), features: vec![], away: None };
        // A group: WhatsApp in French names the owner "Vous"
        let group = |sender: &str, body: &str| RemoteNotification { key: "g".into(), conversation: Some("AUREON".into()), sender: Some(sender.into()), group: true, ..n("com.whatsapp", "WhatsApp", "AUREON", body) };
        inbox.posted(&phone, &group("Chris", "When does the sun go down?"));
        inbox.posted(&phone, &group("Vous", "At around 18:00"));
        let aureon = inbox.list().into_iter().find(|c| c.title == "AUREON").unwrap();
        let thread = inbox.thread(&aureon.id).unwrap();
        assert_eq!((thread[1].mine, thread[1].sender.as_deref()), (true, None));
        assert_eq!(aureon.unread, 0, "answered on the phone");

        // One to one, answered on the phone: the update comes named after the owner
        let jenna = RemoteNotification { key: "j".into(), ..n("com.whatsapp", "WhatsApp", "Jenna", "Bonne nuit") };
        inbox.posted(&phone, &jenna);
        inbox.posted(&phone, &RemoteNotification { conversation: Some("Vous".into()), sender: Some("Vous".into()), title: "Vous".into(), body: "Bonne nuit malala".into(), ..jenna.clone() });
        // And a phone that says so
        inbox.posted(&phone, &RemoteNotification { mine: true, body: "Dors bien".into(), ..jenna.clone() });
        let list = inbox.list();
        assert!(list.iter().all(|c| c.title != "Vous"), "no conversation named after the owner");
        let jenna = list.iter().find(|c| c.title == "Jenna").unwrap();
        let thread: Vec<_> = inbox.thread(&jenna.id).unwrap().into_iter().map(|m| (m.text, m.mine)).collect();
        assert_eq!(thread, [("Bonne nuit".to_string(), false), ("Bonne nuit malala".to_string(), true), ("Dors bien".to_string(), true)]);
        assert_eq!(jenna.last.as_ref().map(|m| m.mine), Some(true));
    }

    #[test]
    fn other_notifications_kept_apart_until_gone() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().into());
        let phone = DeviceInfo { id: Fingerprint::of(b"p"), name: "Pixel".into(), kind: linksy_core::DeviceKind::Phone, os: "android".into(), features: vec![], away: None };
        let quiet = |key: &str, app: &str, title: &str, body: &str| RemoteNotification { key: key.into(), can_reply: false, ..n(app, "App", title, body) };
        assert_eq!(inbox.posted(&phone, &quiet("a", "com.bank", "Sign in", "Your code is 771204")).as_deref(), Some("771204"));
        inbox.posted(&phone, &quiet("b", "com.google.android.calendar", "Team sync", "In 15 minutes"));
        inbox.posted(&phone, &quiet("b", "com.google.android.calendar", "Team sync", "In 5 minutes"));
        inbox.posted(&phone, &n("com.whatsapp", "WhatsApp", "WhatsApp", "12 messages from 3 chats"));
        let alerts = inbox.alerts();
        assert_eq!(alerts.iter().map(|a| (a.key.as_str(), a.body.as_str())).collect::<Vec<_>>(), [("b", "In 5 minutes"), ("a", "Your code is 771204")]);
        assert!(inbox.list().is_empty());
        inbox.removed(&phone.id, "b", false);
        assert_eq!(inbox.alert_keys(&phone.id), ["a"]);
    }

    #[test]
    fn history_fills_in_and_pages_back() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().into());
        let features = vec!["inbox_history".to_string()];
        let phone = DeviceInfo { id: Fingerprint::of(b"p"), name: "Pixel".into(), kind: linksy_core::DeviceKind::Phone, os: "android".into(), features, away: None };
        let said = |sender: Option<&str>, text: &str, time: u64, mine: bool| ConversationMessage { sender: sender.map(Into::into), text: text.into(), time: Some(time), mine };
        let family = RemoteNotification {
            conversation: Some("Family".into()),
            sender: Some("Bob".into()),
            group: true,
            time: Some(5000),
            messages: vec![said(Some("Mum"), "ok", 1000, false), said(None, "ok", 2000, true), said(Some("Mum"), "ok", 3000, false), said(Some("Bob"), "Photos are up", 5000, false)],
            ..n("com.whatsapp", "WhatsApp", "Family", "Bob: Photos are up")
        };
        inbox.posted(&phone, &family);
        let c = inbox.list().remove(0);
        assert_eq!((c.unread, c.older), (1, true), "the history isn't unread, and more can be asked");
        let thread = inbox.thread(&c.id).unwrap();
        let texts: Vec<_> = thread.iter().map(|m| (m.text.as_str(), m.mine, m.at_ms)).collect();
        assert_eq!(texts, [("ok", false, 1000), ("ok", true, 2000), ("ok", false, 3000), ("Photos are up", false, 5000)], "repeated words stay, the newest isn't doubled");

        let (from, app, conversation, before) = inbox.older_than(&c.id).unwrap();
        assert_eq!((from, app.as_str(), conversation.as_str(), before), (phone.id, "com.whatsapp", "Family", 1000));
        inbox.add_older(&phone.id, "com.whatsapp", "Family", &[said(Some("Bob"), "Hi all, your code is 482913", 500, false), said(Some("Mum"), "ok", 1000, false)], false);
        let thread = inbox.thread(&c.id).unwrap();
        assert_eq!(thread.len(), 5);
        assert_eq!((thread[0].code.as_deref(), thread[0].sender.as_deref()), (Some("482913"), Some("Bob")));
        assert!(inbox.older_than(&c.id).is_none(), "the phone said there's nothing older");
    }

    #[test]
    fn a_kept_reply_survives_the_notification() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().into());
        let phone = DeviceInfo { id: Fingerprint::of(b"p"), name: "Pixel".into(), kind: linksy_core::DeviceKind::Phone, os: "android".into(), features: vec![], away: None };
        inbox.posted(&phone, &n("com.whatsapp", "WhatsApp", "Alice", "Lunch?"));
        let id = inbox.list()[0].id.clone();
        inbox.removed(&phone.id, "k", true);
        assert!(inbox.list()[0].can_reply);
        assert_eq!(inbox.notification(&id).map(|(_, key, reply)| (key, reply)), Some(("k".to_string(), true)));
        inbox.removed(&phone.id, "k", false);
        assert!(inbox.notification(&id).is_none());
        assert!(inbox.older_than(&id).is_none(), "a phone without history has none to ask");
        let mail = RemoteNotification { subject: Some("Invoice".into()), ..n("com.google.android.gm", "Gmail", "Shop", "Here it is") };
        inbox.posted(&phone, &mail);
        let shop = inbox.list().into_iter().find(|c| c.title == "Shop").unwrap();
        assert_eq!(shop.last.unwrap().text, "Invoice\nHere it is");
    }
}
