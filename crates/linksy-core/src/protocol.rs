//! Wire protocol. Every connection starts with a handshake on the first bidirectional
//! stream. After that:
//! - each file transfer gets its own bidirectional stream (offer, answer, raw file bytes),
//! - each media session (screen mirroring…) gets its own bidirectional stream too
//!   (request, answer, then media bytes one way and control messages the other),
//! - small one-shot messages (notifications, pings) each get a unidirectional stream.
//!
//! Control messages are JSON, framed with a big-endian u32 length.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::deck_layout::DeckLayouts;
use crate::error::{Result, protocol};
use crate::identity::DeviceInfo;

pub const PROTOCOL_VERSION: u32 = 1;
/// Pairing with commit and reveal (docs/protocol.md §4.4).
pub const PAIRING_VERSION: u32 = 2;
/// The largest message either side takes, in bytes of JSON.
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Reconnect between devices that already trust each other.
    Connect,
    /// Start pairing; both users must confirm the code.
    Pair,
    /// A second connection between paired devices over a direct Wi-Fi link they just set
    /// up (docs/protocol.md §4.17), for big transfers; the first one stays.
    Direct,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Handshake {
    /// `stay_connected`: the sender keeps this link open, so the receiver shouldn't close
    /// it for being idle.
    Hello {
        version: u32,
        device: DeviceInfo,
        intent: Intent,
        #[serde(default)]
        stay_connected: bool,
        /// Token from a pairing invite (QR code) the dialer scanned; pairing then needs no
        /// code comparison.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invite: Option<String>,
        #[serde(default)]
        pairing: u32,
    },
    Welcome {
        device: DeviceInfo,
        #[serde(default)]
        stay_connected: bool,
        #[serde(default)]
        pairing: u32,
    },
    Reject { reason: RejectReason },
    PairCommit { commit: String },
    PairNonce { nonce: String },
    PairDecision { accept: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    NotPaired,
    IncompatibleVersion,
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path with `/` separators. Folders are sent as their files.
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransferMsg {
    /// `import`: photos and videos for the receiver's picture library ("photo
    /// auto-import"), saved without asking when the receiver takes imports.
    Offer {
        id: Uuid,
        files: Vec<FileEntry>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        import: bool,
        /// One file (≤ 10 MiB) for the receiver's clipboard (an image, or a file to paste),
        /// taken without asking.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        clipboard: bool,
        /// Apps among the files, each the APKs in a folder of its own (§4.22).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        apps: Vec<crate::apps::AppInfo>,
    },
    /// `offsets[i]` is how many bytes of file `i` the receiver already has.
    Accept { offsets: Vec<u64> },
    Decline,
    /// Sent after each file's bytes.
    FileDone { blake3: String },
    /// Receiver's final verdict.
    Complete,
    Failed { reason: String },
}

/// What a media session carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// The receiver of the request shares its screen with the requester.
    Screen,
    /// Like `Screen`, and the requester may also use its pointer and keyboard.
    Control,
    /// Sidecar: the *requester* shares a new virtual monitor, and the receiver shows it
    /// full screen and sends pointer and keyboard input back, as a second display.
    Display,
    /// The receiver shares its camera; the requester uses it as a webcam.
    Camera,
    /// The requester (a phone) drives the receiver's pointer and keyboard without seeing
    /// its screen: a touchpad, keyboard or presentation remote. No media; only input.
    Input,
    /// The receiver shares its microphone; the requester uses it as a microphone. Raw
    /// 16-bit 48 kHz mono PCM, little-endian (docs/protocol.md §4.8).
    Microphone,
    /// A shell on the receiver (a computer), shown on the requester (a phone): raw
    /// terminal output one way, `term_*` control messages the other (docs/protocol.md §4.15).
    Terminal,
}

impl SessionKind {
    /// Whether the device that asked for the session is the one sending media.
    pub fn requester_shares(self) -> bool {
        matches!(self, Self::Display)
    }

    /// Whether the requester turns what it receives into a device for its apps (a webcam,
    /// a microphone) rather than showing it.
    pub fn is_device(self) -> bool {
        matches!(self, Self::Camera | Self::Microphone)
    }

    /// Whether the viewing side sends input back.
    pub fn has_input(self) -> bool {
        matches!(self, Self::Control | Self::Display | Self::Input)
    }

    /// Whether a paired device may be allowed it for good, so it starts without asking:
    /// seeing or controlling the screen, or using the mouse and keyboard. Never the
    /// camera, the microphone or a terminal.
    pub fn may_always_allow(self) -> bool {
        matches!(self, Self::Screen | Self::Control | Self::Input)
    }
}

/// Input sent by the viewer of a `control` session, framed like every other message
/// (u32 length + JSON). Positions are fractions of the shared screen (0.0–1.0), so they
/// don't depend on either device's resolution. Keys are X11 keysyms.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMsg {
    Pointer { x: f64, y: f64 },
    Button { button: MouseButton, pressed: bool },
    /// `discrete`: wheel clicks (dx/dy are steps); otherwise smooth scrolling in pixels.
    Scroll { dx: f64, dy: f64, discrete: bool },
    /// `code`: the physical key (Linux evdev code), for a phone driven over the cable, which
    /// reads key positions with its own layout. Optional: older devices send keysyms only.
    Key {
        keysym: u32,
        pressed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<u32>,
    },
    /// Move the pointer by this many pixels (a touchpad); for `input` sessions.
    PointerBy { dx: f64, dy: f64 },
    /// Type this text, as if on a keyboard.
    Text { text: String },
    /// Terminal sessions: what was typed, as the terminal's bytes (xterm sequences).
    TermInput { data: String },
    TermResize { cols: u16, rows: u16 },
    /// End the shell, not just this view of it.
    TermClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

impl MouseButton {
    /// Linux evdev code (BTN_LEFT…), what input APIs take.
    pub fn evdev(self) -> i32 {
        match self {
            Self::Left => 0x110,
            Self::Right => 0x111,
            Self::Middle => 0x112,
            Self::Back => 0x113,
            Self::Forward => 0x114,
        }
    }
}

/// Encode a control message as a frame.
pub fn control_frame(msg: &ControlMsg) -> Vec<u8> {
    let body = serde_json::to_vec(msg).unwrap_or_default();
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend(body);
    out
}

/// Read control frames from a byte stream (blocking), calling `f` for each until the
/// stream ends. Unknown or malformed messages are skipped.
pub fn read_control_frames(mut r: impl std::io::Read, mut f: impl FnMut(ControlMsg)) {
    let mut len = [0u8; 4];
    while r.read_exact(&mut len).is_ok() {
        let n = u32::from_be_bytes(len) as usize;
        if n > MAX_FRAME {
            return;
        }
        let mut body = vec![0u8; n];
        if r.read_exact(&mut body).is_err() {
            return;
        }
        if let Ok(msg) = serde_json::from_slice(&body) {
            f(msg);
        }
    }
}

/// Opening a media session. After `session_accept`, the sharing side writes the media
/// (for `screen`: H.264 Annex-B byte stream) and the requesting side may write framed
/// control messages back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionMsg {
    SessionRequest {
        id: Uuid,
        kind: SessionKind,
        /// For a `terminal`: the code, the size, and which one to reattach.
        #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
        terminal: Option<TerminalOpen>,
        /// For a `screen` or `control`: show the desktop's dialog again, so its user picks
        /// the screen (another monitor), instead of the one picked last time.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        choose: bool,
    },
    /// For a `display`, the accepting device may give its screen's size (landscape), so
    /// the second display is made in its shape.
    SessionAccept {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        /// For a `terminal`: its id, to reattach later.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terminal: Option<String>,
    },
    /// `reason: "unsupported"` when the device can't do this kind of session at all (as
    /// opposed to its user saying no). Absent otherwise.
    SessionDecline {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// A wrong terminal code: how many tries are left before it locks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tries_left: Option<u32>,
    },
}

/// What a phone sends to open a terminal (docs/protocol.md §4.15).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOpen {
    /// The terminal code as typed; only the computer can check it.
    pub code: String,
    pub cols: u16,
    pub rows: u16,
    /// A running terminal to reattach, by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
}

/// A running terminal, as `terminals` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub id: String,
    /// The shell's window title (OSC 0/2), or the shell's name.
    pub title: String,
    pub started_ms: u64,
}

/// A phone plugged into a computer by USB (§4.11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluggedPhone {
    /// Its USB serial, or vendor:product.
    pub key: String,
    /// The USB descriptor strings.
    pub manufacturer: String,
    pub product: String,
    /// `plugged`, `switching`, `accessory` or `failed`.
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Notify { title: String, body: String },
    /// Text for the receiver's clipboard.
    Clipboard { text: String },
    /// The sender's new name, so connected devices update without reconnecting.
    Renamed { name: String },
    /// The sender's features (§4.10), when they change while connected.
    Features { features: Vec<String> },
    /// The key the sender's private discovery tags are made from (§4.16), 64 hex chars.
    /// Sent to paired devices on every connection.
    DiscoveryKey { key: String },
    Ping,
    /// A web link for the receiver to open in its browser ("Open on PC"). `http`/`https`
    /// only; receivers drop anything else.
    OpenUrl { url: String },
    /// The sender's battery, sent when it changes noticeably and on connecting.
    Battery(Battery),
    /// Play a loud sound until `ring_stop` or the user notices ("Ring my phone").
    Ring,
    RingStop,
    /// The phones plugged into the sender (a computer) by USB, sent to connected phones when
    /// that changes and after connecting (§4.11): a phone offers its own Connect button.
    UsbPlugged { phones: Vec<PluggedPhone> },
    /// Use the cable for this plugged-in phone (its `key` from `usb_plugged`).
    UsbConnect { key: String },
    /// A notification on the sender (a phone) to show here; sent again with the same `key`
    /// when it changes.
    NotificationPosted(Box<RemoteNotification>),
    /// That notification is gone on the sender.
    NotificationRemoved {
        key: String,
        /// The phone still answers its conversation through `notification_reply` (§4.5).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reply_kept: bool,
    },
    /// Answer a notification that `can_reply` with this text.
    NotificationReply { key: String, text: String },
    /// Press one of a notification's `actions`.
    NotificationAction { key: String, action: String },
    /// The user dismissed it on the receiving side; dismiss it at the source too.
    NotificationDismiss { key: String },
    /// Everything playing on the sender (all its media players), sent when it changes.
    MediaState(MediaState),
    /// Control one of the receiver's players.
    MediaCommand { player: String, command: MediaCommand },
    /// Start (or stop) sending `media_state` to the sender while connected.
    MediaWatch { on: bool },
    /// The sender's sound output (its system volume), sent when it changes to a device that
    /// asked with `volume_watch`.
    VolumeState(VolumeState),
    /// Change the receiver's volume: `level` (0 to 1), `muted`, or both.
    VolumeSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
    },
    /// Start (or stop) sending `volume_state` to the sender while connected.
    VolumeWatch { on: bool },
    /// A phone's controls (its volumes, ringer, Do Not Disturb and flashlight), sent when they
    /// change to a device that asked with `controls_watch` (docs/protocol.md §4.24).
    ControlsState(ControlsState),
    /// Change some of the receiver's controls (a phone's): only the ones given.
    ControlsSet(ControlsChange),
    /// Start (or stop) sending `controls_state` to the sender while connected.
    ControlsWatch { on: bool },
    /// The receiver didn't change one control (`media`, `ringtone`, `alarm`, `ringer`, `dnd` or
    /// `flashlight`), and says why in words to show as they are.
    ControlsRefused { what: String, reason: String },
    /// The sender (a phone) has a call coming in, going on, or none any more.
    CallState {
        state: CallState,
        /// Who's calling, when the phone may say.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contact: Option<String>,
    },
    /// "Still here": sent every `every_s` seconds to a device that asked with
    /// `heartbeat_watch` (lock when you walk away).
    Heartbeat,
    /// Ask the receiver for a `heartbeat` every `every_s` seconds (`on`), or to stop.
    HeartbeatWatch { on: bool, every_s: u32 },
    /// Ask the receiver to send `clipboard` whenever its clipboard changes (`on`), or to
    /// stop. Not every desktop allows watching it (GNOME doesn't).
    ClipboardWatch { on: bool },
    /// What the camera of a running `camera` session can do and how it's set (phone →
    /// computer), sent when the session starts and after every change; the keys are in
    /// docs/protocol.md §4.8. The computer changes them with `camera_control` frames on the
    /// session itself.
    CameraState { session: uuid::Uuid, state: serde_json::Value },
    /// The sender's own picture (its avatar), a JPEG or PNG ≤ 128 KiB in base64, sent after
    /// connecting; the receiver shows it for that device.
    Picture { image: String },
    /// "Where are you?" (computer → phone, docs/protocol.md §4.14): answered once with
    /// `location` or `location_refused`. Nothing is kept or tracked.
    Locate,
    /// Which terminals are running here for the asker (docs/protocol.md §4.15).
    TerminalList,
    Terminals { list: Vec<TerminalInfo> },
    /// Where the sender is now: degrees, accuracy in metres, when it was measured.
    Location {
        lat: f64,
        lon: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accuracy_m: Option<f64>,
        at_ms: u64,
    },
    /// The sender won't or can't say where it is (turned off, no permission, no fix).
    LocationRefused { reason: String },
    /// "Open your camera and send what you take" (computer → phone, docs/protocol.md §4.23).
    /// The picture comes as an ordinary transfer, which the computer takes without asking.
    SnapRequest,
    /// The phone can't or won't (turned off, no camera permission, the camera is in use).
    SnapRefused { reason: String },
    /// A notification that came up on the computer, for the phone to show too (computer →
    /// phone, docs/protocol.md §4.25). A later one with the same `key` replaces it.
    ComputerNotification {
        key: String,
        /// The app that posted it, as the computer's notification names it.
        app: String,
        title: String,
        #[serde(default)]
        body: String,
        /// When it came up, in milliseconds since the Unix epoch.
        #[serde(default)]
        at_ms: u64,
    },
    /// Texting from the computer. Requests (computer → phone):
    SmsListThreads { limit: u32 },
    SmsListMessages {
        thread: String,
        /// Only messages older than this (paging back); newest first.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_ms: Option<u64>,
        limit: u32,
    },
    /// Send `text` to `to` (one address, or several for a group); `client_id` comes back
    /// in `sms_sent`.
    SmsSend { client_id: String, to: Vec<String>, text: String },
    /// Answers and news (phone → computer):
    SmsThreads { threads: Vec<SmsThread> },
    SmsMessages { thread: String, messages: Vec<SmsMessage>, more: bool },
    /// A message arrived or was sent on the phone (by Linksy or not).
    SmsMessage { thread: String, message: SmsMessage },
    SmsSent {
        client_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Setting up (or ending) a direct Wi-Fi link for big transfers (§4.17).
    Direct { step: DirectStep },
    /// A conversation's messages older than `before_ms`, from what the phone kept (§4.5).
    InboxHistory { app: String, conversation: String, before_ms: u64, limit: u32 },
    InboxHistoryPage { app: String, conversation: String, messages: Vec<ConversationMessage>, more: bool },
    /// The phone's photos changed: one added, deleted… (§4.18)
    GalleryChanged,
    /// "Turn this on, please" (computer → phone, §4.21): `notifications` or `gallery`. The
    /// phone asks its user; the answer is the `features` it announces.
    TurnOn { feature: String },
    /// The phone's deck shows (the computer sends `deck_actions`, then `deck_state` as it
    /// changes), or stopped showing (§4.19).
    DeckOpen,
    DeckClose,
    DeckRun {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
    },
    DeckActions { actions: Vec<DeckAction> },
    DeckState { states: Vec<DeckState> },
    /// The deck's layouts (§4.20), computer → phone, only to a phone with `deck_layout`.
    DeckLayout { layouts: DeckLayouts },
}

/// The steps of a direct link (docs/protocol.md §4.17), between a device that hosts it
/// (feature `direct_host`: a phone, with Wi-Fi Direct) and one that joins it
/// (`direct_join`: a computer, or another phone). Addresses are Wi-Fi Direct device
/// addresses (`aa:bb:cc:dd:ee:ff`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DirectStep {
    /// Joiner → host: a direct link would help, start one (or answer `end`). `frequency` is the
    /// channel (MHz) of the joiner's Wi-Fi, so the host can make its group there: a Wi-Fi chip
    /// serving its network and the group on two channels hops between them, and a transfer that
    /// runs at 79 MB/s on one channel runs at 17 on two.
    Ask {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frequency: Option<u32>,
    },
    /// Host → joiner: the host's address and its Wi-Fi Direct name; become findable and
    /// answer `listening`. Android doesn't tell apps their own address (they get
    /// `02:00:00:00:00:00`), so the joiner finds the host by name then. With `group` (both
    /// have `direct_group`), the host made a group with these credentials instead: join
    /// it and wait for `up`, no `listening` or `connecting`.
    Invite {
        address: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<DirectGroup>,
    },
    /// Joiner → host: findable at this address.
    Listening { address: String },
    /// Host → joiner: the host is connecting to it now; connect back to the host's address.
    Connecting,
    /// Host → joiner: the link is up; the host's Linksy is at `ip`:`port` on it.
    Up { ip: std::net::IpAddr, port: u16 },
    /// Either way: it failed (`reason`) or is no longer needed; both drop the link.
    End {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Either way: a send is being prepared, so don't close the link for idleness for
    /// `seconds` more (at most 180). Handled by the core; an older device ignores it.
    Keep { seconds: u32 },
}

impl DirectStep {
    /// Whether what it carries looks right: a device address, a private IPv4 address (or
    /// this one's, for tests).
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Invite { address, name, group } => {
                is_device_address(address) && name.as_ref().is_none_or(|n| n.len() <= 64) && group.as_ref().is_none_or(DirectGroup::is_valid)
            }
            Self::Listening { address } => is_device_address(address),
            Self::Up { ip: std::net::IpAddr::V4(ip), port } => (ip.is_private() || ip.is_link_local() || ip.is_loopback()) && *port != 0,
            Self::Up { .. } => false,
            Self::End { reason } => reason.as_ref().is_none_or(|r| r.len() <= 200),
            Self::Ask { frequency } => frequency.is_none_or(|f| (2400..=7125).contains(&f)),
            Self::Connecting | Self::Keep { .. } => true,
        }
    }
}

/// A Wi-Fi Direct group the host makes with a name and passphrase, which the joiner joins
/// with them: no address needed, and no prompt on Android 10 and later (phone to phone).
/// The passphrase only ever travels inside the paired devices' TLS connection; it's left
/// out of `Debug`, so logs never show it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectGroup {
    /// `DIRECT-xy…`, at most 32 bytes.
    pub ssid: String,
    /// WPA2: 8 to 63 printable ASCII characters.
    pub passphrase: String,
    /// The group's frequency in MHz once it's up, so the joiner looks there first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<u32>,
}

impl DirectGroup {
    /// New credentials: `DIRECT-xy-Linksy` and a 20-character passphrase, random.
    pub fn random() -> std::io::Result<Self> {
        const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
        let mut bytes = [0u8; 22];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        let mut pick = bytes.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char);
        let ssid = format!("DIRECT-{}{}-Linksy", pick.next().unwrap_or('L'), pick.next().unwrap_or('k'));
        Ok(Self { ssid, passphrase: pick.collect(), frequency: None })
    }

    pub fn is_valid(&self) -> bool {
        let ssid = self.ssid.len() <= 32 && self.ssid.starts_with("DIRECT-") && self.ssid.len() >= "DIRECT-xy".len();
        let passphrase = (8..=63).contains(&self.passphrase.len()) && self.passphrase.bytes().all(|b| (0x20..=0x7e).contains(&b));
        ssid && passphrase && self.frequency.is_none_or(|f| (2400..=7125).contains(&f))
    }
}

impl std::fmt::Debug for DirectGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectGroup").field("ssid", &self.ssid).field("passphrase", &"…").field("frequency", &self.frequency).finish()
    }
}

/// `aa:bb:cc:dd:ee:ff`, hex in either case.
fn is_device_address(s: &str) -> bool {
    s.len() == 17 && s.split(':').all(|b| b.len() == 2 && b.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// Browsing a device's files ("phone in the Files sidebar"): the first message of a
/// bidirectional stream, one request per stream. Paths are absolute within what the
/// serving device shares ("/" is its root), with `/` separators.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FsRequest {
    /// A folder's entries → `fs_entries`.
    List { path: String },
    /// One entry → `fs_entry`.
    Stat { path: String },
    /// Up to `length` bytes from `offset` → `fs_data` and then exactly that many raw bytes.
    Read { path: String, offset: u64, length: u64 },
    /// `length` raw bytes follow the request, to write at `offset` (`truncate`: the file
    /// ends after them; creates it) → `fs_ok`.
    Write { path: String, offset: u64, length: u64, truncate: bool },
    Mkdir { path: String },
    /// A file, or a folder with everything in it.
    Delete { path: String },
    Rename { path: String, to: String },
    /// What's shared, to label it ("Pixel 9 (Documents)") → `fs_info`. Optional: a device
    /// that doesn't know it answers `fs_error`.
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsReply {
    FsEntries { entries: Vec<FsEntry> },
    FsEntry { entry: FsEntry },
    FsData { length: u64 },
    FsOk,
    /// The shared folder's name, for labels.
    FsInfo { name: String },
    FsError {
        message: String,
        #[serde(default)]
        not_found: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub dir: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified_ms: u64,
}

/// A conversation, newest first in lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsThread {
    pub id: String,
    /// Phone numbers, and what the phone's contacts call them (same order; may be empty).
    pub addresses: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub snippet: String,
    pub date_ms: u64,
    #[serde(default)]
    pub unread: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsMessage {
    pub id: String,
    pub from_me: bool,
    /// Who sent it (for messages from others, in groups).
    #[serde(default)]
    pub address: String,
    pub body: String,
    pub date_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    Ringing,
    Active,
    Idle,
}

/// A device's media players, as a whole: a later `seq` replaces an earlier one (messages
/// can arrive out of order), and a player missing from it has gone. `seq` is the time it
/// was taken, in milliseconds since 1970, so it keeps growing across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaState {
    pub seq: u64,
    pub players: Vec<MediaPlayer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPlayer {
    /// Stable id on the sender, for commands.
    pub id: String,
    /// The app playing ("Spotify").
    pub app: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    pub playing: bool,
    /// Where it is and how long it is, in milliseconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_ms: Option<u64>,
    #[serde(default)]
    pub can_next: bool,
    #[serde(default)]
    pub can_previous: bool,
    #[serde(default)]
    pub can_seek: bool,
    /// Cover art, a small JPEG or PNG (≤ 64 KiB) in standard base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
}

/// A device's sound output: its system volume, from 0 (silent) to 1 (100 %), and whether
/// it's muted. `seq` as in [`MediaState`]: a later one replaces an earlier one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumeState {
    pub seq: u64,
    pub level: f64,
    pub muted: bool,
}

/// How a phone makes sound for calls and notifications.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ringer {
    #[default]
    Normal,
    Vibrate,
    Silent,
    /// A value a newer Linksy knows.
    #[serde(other)]
    Unknown,
}

/// A phone's Do Not Disturb: what still gets through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dnd {
    /// Everything does.
    #[default]
    Off,
    /// What the phone's own Do Not Disturb settings let through.
    Priority,
    /// Only alarms.
    Alarms,
    /// Nothing.
    Silence,
    /// A value a newer Linksy knows.
    #[serde(other)]
    Unknown,
}

/// A phone's controls. Levels run from 0 to 1 of each stream's range. `seq` as in
/// [`MediaState`]: a later one replaces an earlier one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlsState {
    pub seq: u64,
    pub media: f64,
    pub ringtone: f64,
    pub alarm: f64,
    pub ringer: Ringer,
    pub dnd: Dnd,
    /// Android lets Linksy change Do Not Disturb (the user gave it Do Not Disturb access).
    #[serde(default)]
    pub dnd_access: bool,
    /// The flashlight is on; absent when the phone has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flashlight: Option<bool>,
}

/// What to change in a phone's controls: only what is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlsChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ringtone: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ringer: Option<Ringer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dnd: Option<Dnd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flashlight: Option<bool>,
}

impl ControlsChange {
    /// The same with every level kept within 0 to 1 and anything that isn't a number dropped.
    pub fn tidy(mut self) -> Self {
        let level = |l: Option<f64>| l.filter(|l| l.is_finite()).map(|l| l.clamp(0.0, 1.0));
        (self.media, self.ringtone, self.alarm) = (level(self.media), level(self.ringtone), level(self.alarm));
        self.ringer = self.ringer.filter(|r| *r != Ringer::Unknown);
        self.dnd = self.dnd.filter(|d| *d != Dnd::Unknown);
        self
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MediaCommand {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    /// Jump to this position.
    Seek { position_ms: u64 },
}

/// A notification mirrored from another device (a phone's WhatsApp message…).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteNotification {
    /// Stable id on the sender (Android: the StatusBarNotification key).
    pub key: String,
    /// The app's id (Android package name), for muting.
    pub app: String,
    pub app_name: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Whether `notification_reply` works on it.
    #[serde(default)]
    pub can_reply: bool,
    /// Buttons the notification has, besides replying.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NotificationButton>,
    /// The app's icon, a small PNG (≤ 64 KiB) in standard base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    // Messaging details, all optional (docs/protocol.md §4.5)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub group: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The conversation's latest messages, oldest first, when the app keeps them in its
    /// notification; `body` is still the newest alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<ConversationMessage>,
    /// A mail's subject, with its text in `body`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The newest message (`body`) is the phone owner's own, written on the phone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mine: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<u64>,
    /// Sent by the phone's owner.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mine: bool,
}

/// What a phone's gallery is asked, one request per bidirectional stream (§4.18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GalleryRequest {
    /// Its albums → `gallery_albums`.
    Albums,
    /// Photos and videos, newest first: an album's (all of them without one), a page at a
    /// time, each page older than `before` → `gallery_items`.
    Items {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        album: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<GalleryCursor>,
        limit: u32,
    },
    /// A JPEG of one, `size` px on its longer side → `gallery_data` and its bytes.
    Thumb { id: String, size: u32 },
    /// Send these to the asking device as a transfer with this id (`import`) → `gallery_ok`.
    Send { ids: Vec<String>, transfer: uuid::Uuid },
}

/// Where a page of items ended: the last one's time and id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalleryCursor {
    pub taken_ms: u64,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalleryAlbum {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalleryItem {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub video: bool,
    pub taken_ms: u64,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GalleryReply {
    GalleryAlbums { albums: Vec<GalleryAlbum> },
    GalleryItems { items: Vec<GalleryItem>, more: bool },
    GalleryData { length: u64 },
    GalleryOk,
    GalleryError { message: String },
}

/// Something the computer lets a phone's deck do (§4.19).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeckAction {
    pub id: String,
    pub name: String,
    pub kind: DeckKind,
    /// Which list it belongs to ("Media", "Sound", "System", "Network", "Power", "Commands").
    #[serde(default)]
    pub group: String,
    /// The question the phone asks before running it ("Restart Office PC?").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeckKind {
    Button,
    Toggle,
    /// Takes a `value` from 0 to 1.
    Slider,
}

/// An action's state now, for the deck's tiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeckState {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// What it shows, like the sound output's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationButton {
    pub id: String,
    pub label: String,
}

/// A device's battery as it reports it: `level` in percent (0–100).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub level: u8,
    pub charging: bool,
}

pub async fn write_msg<T: Serialize>(s: &mut quinn::SendStream, msg: &T) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    // The other side would refuse it
    if body.len() > MAX_FRAME {
        return Err(protocol(format!("a message of {} bytes is over the limit", body.len())));
    }
    s.write_all(&(body.len() as u32).to_be_bytes()).await?;
    s.write_all(&body).await?;
    Ok(())
}

/// Read one frame without knowing its type yet (the first message on a new stream).
pub async fn read_value(s: &mut quinn::RecvStream) -> Result<serde_json::Value> {
    read_msg(s).await
}

pub async fn read_msg<T: for<'de> Deserialize<'de>>(s: &mut quinn::RecvStream) -> Result<T> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(protocol(format!("frame of {len} bytes exceeds limit")));
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {

    #[test]
    fn terminal_requests_round_trip() {
        let id = Uuid::nil();
        let open = TerminalOpen { code: "482913".into(), cols: 80, rows: 24, resume: None };
        let msg = SessionMsg::SessionRequest { id, kind: SessionKind::Terminal, terminal: Some(open), choose: false };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""code":"482913""#) && json.contains(r#""kind":"terminal""#), "{json}");
        assert_eq!(serde_json::from_str::<SessionMsg>(&json).unwrap(), msg);
        // As the node reads it: a JSON value first.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::from_value::<SessionMsg>(value).unwrap(), msg);
        // Other sessions carry no terminal part.
        let screen = SessionMsg::SessionRequest { id, kind: SessionKind::Screen, terminal: None, choose: false };
        let json = serde_json::to_string(&screen).unwrap();
        assert!(!json.contains("choose"), "{json}");
        assert_eq!(serde_json::from_str::<SessionMsg>(&json).unwrap(), screen);
        assert_eq!(serde_json::from_str::<SessionMsg>(r#"{"type":"session_request","id":"00000000-0000-0000-0000-000000000000","kind":"screen"}"#).unwrap(), screen);
        // Picking the screen again
        let choose = SessionMsg::SessionRequest { id, kind: SessionKind::Control, terminal: None, choose: true };
        let json = r#"{"type":"session_request","id":"00000000-0000-0000-0000-000000000000","kind":"control","choose":true}"#;
        assert_eq!(serde_json::from_str::<SessionMsg>(json).unwrap(), choose);
        assert_eq!(serde_json::from_value::<SessionMsg>(serde_json::from_str(json).unwrap()).unwrap(), choose);
    }

    use super::*;

    #[test]
    fn volume_messages_read_as_documented() {
        let state: Message = serde_json::from_str(r#"{"type":"volume_state","seq":5,"level":0.4,"muted":false}"#).unwrap();
        assert_eq!(state, Message::VolumeState(VolumeState { seq: 5, level: 0.4, muted: false }));
        // Either half alone
        let set: Message = serde_json::from_str(r#"{"type":"volume_set","muted":true}"#).unwrap();
        assert_eq!(set, Message::VolumeSet { level: None, muted: Some(true) });
        assert_eq!(serde_json::to_string(&Message::VolumeSet { level: Some(0.5), muted: None }).unwrap(), r#"{"type":"volume_set","level":0.5}"#);
        assert_eq!(serde_json::from_str::<Message>(r#"{"type":"volume_watch","on":true}"#).unwrap(), Message::VolumeWatch { on: true });
    }

    #[test]
    fn turn_on_reads_as_documented() {
        let ask = Message::TurnOn { feature: "gallery".into() };
        assert_eq!(serde_json::to_string(&ask).unwrap(), r#"{"type":"turn_on","feature":"gallery"}"#);
        assert_eq!(serde_json::from_str::<Message>(r#"{"type":"turn_on","feature":"gallery"}"#).unwrap(), ask);
    }

    #[test]
    fn direct_steps_read_as_documented() {
        let invite: Message = serde_json::from_str(r#"{"type":"direct","step":{"type":"invite","address":"AA:bb:cc:dd:ee:01"}}"#).unwrap();
        assert_eq!(invite, Message::Direct { step: DirectStep::Invite { address: "AA:bb:cc:dd:ee:01".into(), name: None, group: None } });
        let named: Message = serde_json::from_str(r#"{"type":"direct","step":{"type":"invite","address":"02:00:00:00:00:00","name":"realme 15 Pro 5G"}}"#).unwrap();
        assert_eq!(named, Message::Direct { step: DirectStep::Invite { address: "02:00:00:00:00:00".into(), name: Some("realme 15 Pro 5G".into()), group: None } });
        let up = Message::Direct { step: DirectStep::Up { ip: "192.168.49.1".parse().unwrap(), port: 47200 } };
        assert_eq!(serde_json::to_string(&up).unwrap(), r#"{"type":"direct","step":{"type":"up","ip":"192.168.49.1","port":47200}}"#);
        assert_eq!(serde_json::from_str::<Message>(r#"{"type":"direct","step":{"type":"connecting"}}"#).unwrap(), Message::Direct { step: DirectStep::Connecting });
        assert_eq!(serde_json::to_string(&Message::Direct { step: DirectStep::End { reason: None } }).unwrap(), r#"{"type":"direct","step":{"type":"end"}}"#);
        assert_eq!(serde_json::to_string(&Intent::Direct).unwrap(), r#""direct""#);

        assert!(DirectStep::Listening { address: "02:00:00:00:00:0a".into() }.is_valid());
        assert!(!DirectStep::Listening { address: "02:00:00:00:00:0".into() }.is_valid());
        assert!(!DirectStep::Invite { address: "02:00:00:00:00:zz".into(), name: None, group: None }.is_valid());
        assert!(!DirectStep::Invite { address: "02:00:00:00:00:00".into(), name: Some("x".repeat(65)), group: None }.is_valid());
        // Only a private address on the link: never somewhere on the internet
        assert!(!DirectStep::Up { ip: "8.8.8.8".parse().unwrap(), port: 47200 }.is_valid());
        assert!(!DirectStep::Up { ip: "192.168.49.1".parse().unwrap(), port: 0 }.is_valid());
        assert!(!DirectStep::Up { ip: "::1".parse().unwrap(), port: 47200 }.is_valid());
    }

    #[test]
    fn direct_groups_round_trip_check_their_credentials_and_hide_the_passphrase() {
        let group = DirectGroup::random().unwrap();
        assert!(group.is_valid(), "{group:?}");
        assert!(group.ssid.starts_with("DIRECT-") && group.ssid.ends_with("-Linksy") && group.passphrase.len() == 20);
        assert_ne!(DirectGroup::random().unwrap().passphrase, group.passphrase);

        let json = r#"{"type":"direct","step":{"type":"invite","address":"02:00:00:00:00:00","name":"Pixel 9","group":{"ssid":"DIRECT-k4-Linksy","passphrase":"correct horse battery","frequency":5180}}}"#;
        let invite: Message = serde_json::from_str(json).unwrap();
        let Message::Direct { step: step @ DirectStep::Invite { group: Some(_), .. } } = &invite else { panic!("{invite:?}") };
        assert!(step.is_valid());
        assert_eq!(serde_json::to_string(&invite).unwrap(), json);
        // Never in logs
        assert!(!format!("{invite:?}").contains("horse"));

        let ok = DirectGroup { ssid: "DIRECT-k4-Linksy".into(), passphrase: "12345678".into(), frequency: None };
        assert!(ok.is_valid());
        let bad = [
            DirectGroup { ssid: "Linksy".into(), ..ok.clone() },
            DirectGroup { ssid: "DIRECT-".into(), ..ok.clone() },
            DirectGroup { ssid: format!("DIRECT-{}", "x".repeat(26)), ..ok.clone() },
            DirectGroup { passphrase: "1234567".into(), ..ok.clone() },
            DirectGroup { passphrase: "x".repeat(64), ..ok.clone() },
            DirectGroup { passphrase: "pass\nword1".into(), ..ok.clone() },
            DirectGroup { passphrase: "pässword".into(), ..ok.clone() },
            DirectGroup { frequency: Some(900), ..ok.clone() },
        ];
        for group in bad {
            assert!(!group.is_valid(), "{group:?}");
            assert!(!DirectStep::Invite { address: "02:00:00:00:00:00".into(), name: None, group: Some(group) }.is_valid());
        }
    }

    #[test]
    fn control_frames_round_trip_and_skip_junk() {
        let msgs = [
            ControlMsg::Pointer { x: 0.25, y: 0.75 },
            ControlMsg::Button { button: MouseButton::Right, pressed: true },
            ControlMsg::Scroll { dx: 0.0, dy: -2.0, discrete: true },
            ControlMsg::Key { keysym: 0xff0d, pressed: false, code: None },
        ];
        let mut bytes = Vec::new();
        for (i, m) in msgs.iter().enumerate() {
            bytes.extend(control_frame(m));
            if i == 1 {
                // A message from a newer version: skipped, the rest still read.
                let junk = br#"{"type":"teleport"}"#;
                bytes.extend((junk.len() as u32).to_be_bytes());
                bytes.extend(junk);
            }
        }
        let mut got = Vec::new();
        read_control_frames(&bytes[..], |m| got.push(m));
        assert_eq!(got, msgs);
        assert_eq!(serde_json::to_string(&msgs[1]).unwrap(), r#"{"type":"button","button":"right","pressed":true}"#);
    }

    #[test]
    fn deck_messages_old_and_new() {
        let older = r#"{"type":"deck_actions","actions":[{"id":"media.next","name":"Next Track","kind":"button","group":"Media"}]}"#;
        let Message::DeckActions { actions } = serde_json::from_str(older).unwrap() else { panic!() };
        assert_eq!(actions[0].confirm, None);
        let restart = DeckAction { id: "system.restart".into(), name: "Restart".into(), kind: DeckKind::Button, group: "System".into(), confirm: Some("Restart Office PC?".into()) };
        let json = serde_json::to_string(&Message::DeckActions { actions: vec![restart.clone()] }).unwrap();
        assert!(json.contains(r#""confirm":"Restart Office PC?""#), "{json}");
        let state = DeckState { id: "sound.output".into(), on: None, value: None, text: Some("Speakers".into()) };
        assert_eq!(serde_json::to_string(&state).unwrap(), r#"{"id":"sound.output","text":"Speakers"}"#);
        // What a phone built before these fields does: they're ignored
        #[derive(Deserialize)]
        struct Before {
            id: String,
        }
        let before: Before = serde_json::from_str(&serde_json::to_string(&restart).unwrap()).unwrap();
        assert_eq!(before.id, "system.restart");
        // A kind it doesn't know fails the whole list, so the computer adds none
        assert!(serde_json::from_str::<Message>(&json.replace(r#""kind":"button""#, r#""kind":"dial""#)).is_err());
    }

    #[test]
    fn snap_messages_have_the_names_the_phone_is_told() {
        assert_eq!(serde_json::to_string(&Message::SnapRequest).unwrap(), r#"{"type":"snap_request"}"#);
        let refused = serde_json::to_string(&Message::SnapRefused { reason: "Linksy needs the camera permission".into() }).unwrap();
        assert_eq!(refused, r#"{"type":"snap_refused","reason":"Linksy needs the camera permission"}"#);
        assert!(matches!(serde_json::from_str::<Message>(&refused), Ok(Message::SnapRefused { .. })));
    }

    #[test]
    fn computer_notifications_have_the_names_the_phone_is_told() {
        let shown = Message::ComputerNotification { key: "7".into(), app: "Thunderbird".into(), title: "Mum".into(), body: "Dinner?".into(), at_ms: 1760945000123 };
        let json = serde_json::to_string(&shown).unwrap();
        assert_eq!(json, r#"{"type":"computer_notification","key":"7","app":"Thunderbird","title":"Mum","body":"Dinner?","at_ms":1760945000123}"#);
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), shown);
        // A notification with no body or time is still one
        let bare: Message = serde_json::from_str(r#"{"type":"computer_notification","key":"7","app":"Clock","title":"Alarm"}"#).unwrap();
        assert!(matches!(bare, Message::ComputerNotification { ref body, at_ms: 0, .. } if body.is_empty()));
    }

    #[test]
    fn phone_controls_messages_have_the_names_the_phone_is_told() {
        let set = Message::ControlsSet(ControlsChange { media: Some(0.5), ringer: Some(Ringer::Vibrate), dnd: Some(Dnd::Priority), flashlight: Some(true), ..Default::default() });
        assert_eq!(serde_json::to_string(&set).unwrap(), r#"{"type":"controls_set","media":0.5,"ringer":"vibrate","dnd":"priority","flashlight":true}"#);
        assert_eq!(serde_json::to_string(&Message::ControlsWatch { on: true }).unwrap(), r#"{"type":"controls_watch","on":true}"#);
        let state = Message::ControlsState(ControlsState { seq: 7, media: 0.4, ringtone: 0.8, alarm: 1.0, ringer: Ringer::Normal, dnd: Dnd::Off, dnd_access: true, flashlight: Some(false) });
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            r#"{"type":"controls_state","seq":7,"media":0.4,"ringtone":0.8,"alarm":1.0,"ringer":"normal","dnd":"off","dnd_access":true,"flashlight":false}"#
        );
        assert_eq!(serde_json::from_str::<Message>(&serde_json::to_string(&state).unwrap()).unwrap(), state);
        let refused = serde_json::to_string(&Message::ControlsRefused { what: "dnd".into(), reason: "Allow Do Not Disturb access to Linksy on the phone first".into() }).unwrap();
        assert_eq!(refused, r#"{"type":"controls_refused","what":"dnd","reason":"Allow Do Not Disturb access to Linksy on the phone first"}"#);
    }

    #[test]
    fn a_phone_without_a_flashlight_or_access_and_values_from_a_newer_linksy() {
        let older = r#"{"type":"controls_state","seq":1,"media":0.1,"ringtone":0.2,"alarm":0.3,"ringer":"silent","dnd":"alarms"}"#;
        let Message::ControlsState(s) = serde_json::from_str(older).unwrap() else { panic!() };
        assert_eq!((s.flashlight, s.dnd_access, s.ringer, s.dnd), (None, false, Ringer::Silent, Dnd::Alarms));
        // A value this version doesn't know is read as Unknown, not as a broken message
        let newer = older.replace(r#""ringer":"silent""#, r#""ringer":"loud""#).replace(r#""dnd":"alarms""#, r#""dnd":"gaming""#);
        let Message::ControlsState(s) = serde_json::from_str(&newer).unwrap() else { panic!() };
        assert_eq!((s.ringer, s.dnd), (Ringer::Unknown, Dnd::Unknown));
        // And such a value is never passed on as a change
        let change = ControlsChange { media: Some(7.0), alarm: Some(f64::NAN), ringer: Some(Ringer::Unknown), dnd: Some(Dnd::Unknown), ..Default::default() }.tidy();
        assert_eq!((change.media, change.alarm, change.ringer, change.dnd), (Some(1.0), None, None, None));
        assert!(ControlsChange::default().is_empty());
    }

    #[test]
    fn ask_carries_the_joiners_channel_only_when_it_has_one() {
        assert_eq!(serde_json::to_string(&Message::Direct { step: DirectStep::Ask { frequency: None } }).unwrap(), r#"{"type":"direct","step":{"type":"ask"}}"#);
        assert_eq!(serde_json::to_string(&Message::Direct { step: DirectStep::Ask { frequency: Some(5200) } }).unwrap(), r#"{"type":"direct","step":{"type":"ask","frequency":5200}}"#);
        // What an older computer sends, and what an older phone does with a newer one
        assert!(matches!(serde_json::from_str::<Message>(r#"{"type":"direct","step":{"type":"ask"}}"#), Ok(Message::Direct { step: DirectStep::Ask { frequency: None } })));
        assert!(!DirectStep::Ask { frequency: Some(80) }.is_valid());
    }
}
