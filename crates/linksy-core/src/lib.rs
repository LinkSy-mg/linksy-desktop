//! Linksy core: device identity, discovery, pairing and file transfer over QUIC.
//! UI-independent; the daemon and every native front end build on [`Node`].

pub mod away;
mod discovery;
pub mod invite;
pub mod error;
pub mod fs;
pub mod identity;
mod node;
pub mod protocol;
mod settings;
mod tls;
pub mod transfer;
pub mod trust;

pub use error::{Error, Result};
pub use identity::{DeviceInfo, DeviceKind, Fingerprint};
pub use node::{Config, DeviceView, Direction, Event, MAX_CLIPBOARD, Node, Session, over_usb, Link, link_of};
pub use protocol::{Battery, CallState, ControlMsg, FileEntry, FsEntry, FsReply, FsRequest, MediaCommand, MediaPlayer, MediaState, MouseButton, NotificationButton, RemoteNotification, SessionKind, SmsMessage, SmsThread, TerminalInfo, TerminalOpen};
pub use node::{TerminalCall, Verdict};
pub use settings::{MAX_NAME, Settings};
pub use trust::TrustedDevice;
pub use uuid::Uuid;
