//! Linksy core: device identity, discovery, pairing and file transfer over QUIC.
//! UI-independent; the daemon and every native front end build on [`Node`].

mod discovery;
pub mod invite;
pub mod error;
pub mod identity;
mod node;
pub mod protocol;
mod settings;
mod tls;
pub mod transfer;
pub mod trust;

pub use error::{Error, Result};
pub use identity::{DeviceInfo, DeviceKind, Fingerprint};
pub use node::{Config, DeviceView, Direction, Event, MAX_CLIPBOARD, Node, Session};
pub use protocol::{Battery, ControlMsg, FileEntry, MediaCommand, MediaPlayer, MediaState, MouseButton, NotificationButton, RemoteNotification, SessionKind};
pub use settings::{MAX_NAME, Settings};
pub use trust::TrustedDevice;
pub use uuid::Uuid;
