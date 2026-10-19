//! Wire protocol. Every connection starts with a handshake on the first bidirectional
//! stream. After that:
//! - each file transfer gets its own bidirectional stream (offer, answer, raw file bytes),
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Notify { title: String, body: String },
    /// Text for the receiver's clipboard.
    Clipboard { text: String },
    Ping,
}

pub async fn write_msg<T: Serialize>(s: &mut quinn::SendStream, msg: &T) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    s.write_all(&(body.len() as u32).to_be_bytes()).await?;
    s.write_all(&body).await?;
    Ok(())
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
