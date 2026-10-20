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

use crate::error::{Result, protocol};
use crate::identity::DeviceInfo;

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Reconnect between devices that already trust each other.
    Connect,
    /// Start pairing; both users must confirm the code.
    Pair,
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
    },
    Welcome {
        device: DeviceInfo,
        #[serde(default)]
        stay_connected: bool,
    },
    Reject { reason: RejectReason },
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
    Offer { id: Uuid, files: Vec<FileEntry> },
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
}

impl SessionKind {
    /// Whether the device that asked for the session is the one sending media.
    pub fn requester_shares(self) -> bool {
        matches!(self, Self::Display)
    }

    /// Whether the viewing side sends input back.
    pub fn has_input(self) -> bool {
        matches!(self, Self::Control | Self::Display)
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
    Key { keysym: u32, pressed: bool },
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
    SessionRequest { id: Uuid, kind: SessionKind },
    SessionAccept,
    /// `reason: "unsupported"` when the device can't do this kind of session at all (as
    /// opposed to its user saying no). Absent otherwise.
    SessionDecline {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Notify { title: String, body: String },
    /// Text for the receiver's clipboard.
    Clipboard { text: String },
    /// The sender's new name, so connected devices update without reconnecting.
    Renamed { name: String },
    Ping,
    /// A web link for the receiver to open in its browser ("Open on PC"). `http`/`https`
    /// only; receivers drop anything else.
    OpenUrl { url: String },
    /// The sender's battery, sent when it changes noticeably and on connecting.
    Battery(Battery),
    /// Play a loud sound until `ring_stop` or the user notices ("Ring my phone").
    Ring,
    RingStop,
    /// A notification on the sender (a phone) to show here; sent again with the same `key`
    /// when it changes.
    NotificationPosted(RemoteNotification),
    /// That notification is gone on the sender.
    NotificationRemoved { key: String },
    /// Answer a notification that `can_reply` with this text.
    NotificationReply { key: String, text: String },
    /// Press one of a notification's `actions`.
    NotificationAction { key: String, action: String },
    /// The user dismissed it on the receiving side; dismiss it at the source too.
    NotificationDismiss { key: String },
}

/// A notification mirrored from another device (a phone's WhatsApp message…).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    use super::*;

    #[test]
    fn control_frames_round_trip_and_skip_junk() {
        let msgs = [
            ControlMsg::Pointer { x: 0.25, y: 0.75 },
            ControlMsg::Button { button: MouseButton::Right, pressed: true },
            ControlMsg::Scroll { dx: 0.0, dy: -2.0, discrete: true },
            ControlMsg::Key { keysym: 0xff0d, pressed: false },
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
}
