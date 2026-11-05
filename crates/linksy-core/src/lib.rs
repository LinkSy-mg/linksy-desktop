//! Linksy core: device identity, discovery, pairing and file transfer over QUIC.
//! UI-independent; the daemon and every native front end build on [`Node`].

pub mod apps;
pub mod away;
pub mod cable;
pub mod deck_layout;
mod discovery;
pub mod invite;
pub mod error;
pub mod fs;
pub mod gallery;
pub mod identity;
mod node;
pub mod preflight;
pub mod protocol;
pub mod route;
mod settings;
pub mod space;
mod tls;
pub mod transfer;
pub mod trust;

pub use deck_layout::{DeckFace, DeckLayout, DeckLayouts, DeckTheme, DeckTile};
pub use error::{Error, Result};
pub use identity::{DeviceInfo, DeviceKind, Fingerprint};
pub use node::{Config, DeviceView, Direction, Event, MAX_CLIPBOARD, Node, Session, folder_name, over_usb, Link, link_of};
pub use protocol::{
    Battery, CallState, ControlMsg, ConversationMessage, DeckAction, DeckKind, DeckState, DirectGroup, DirectStep, FileEntry, FsEntry, FsReply, FsRequest, GalleryAlbum, GalleryCursor, GalleryItem, GalleryReply, GalleryRequest, MediaCommand, MediaPlayer,
    MediaState, MouseButton, NotificationButton, PluggedPhone, RemoteNotification, SessionKind, SmsMessage, SmsThread, TerminalInfo, TerminalOpen, VolumeState,
};
pub use node::{TerminalCall, Verdict};
pub use apps::{AppHave, AppInfo, AppProbe, AppSend, AppState, DirectReady, PreflightMsg, PreflightReply, ReceivedApp};
pub use preflight::{PreflightAnswer, PreflightCall};
pub use route::{DirectPlan, Route, SendPlan, Skip};
pub use settings::{MAX_NAME, Settings};
pub use trust::TrustedDevice;
pub use uuid::Uuid;
