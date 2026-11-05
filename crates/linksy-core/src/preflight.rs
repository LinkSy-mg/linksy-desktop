//! One round trip before a send (docs/protocol.md §4.22): is there room, will it ask,
//! can it do a direct link, which of these apps does it have. The sending device calls
//! [`Node::preflight`]; the answering device's core replies on its own with what it knows
//! (room, whether it asks) and lets its app add the rest through [`Node::incoming_preflight`].

use std::time::Duration;

use tokio::sync::oneshot;

use crate::apps::{AppProbe, AppState, DirectReady, MAX_PROBES, PreflightMsg, PreflightReply};
use crate::error::{Error, Result, protocol};
use crate::identity::{DeviceInfo, Fingerprint};
use crate::node::Node;
use crate::protocol::{read_msg, write_msg};

/// How long the asking device waits for the answer.
const ASK_TIMEOUT: Duration = Duration::from_secs(6);

/// What an app adds to a preflight answer; what it leaves out the core fills in.
#[derive(Clone, Debug, Default)]
pub struct PreflightAnswer {
    /// Room where received files really end up, when that isn't where the core puts them
    /// (a phone that copies what it receives somewhere else needs room for both).
    pub free: Option<u64>,
    pub direct: Option<DirectReady>,
    pub can_install: Option<bool>,
    pub apps: Vec<AppState>,
}

/// A preflight from another device, to answer once, soon: the asker gives up after a few
/// seconds, and the core answers with its own facts if the app hasn't.
pub struct PreflightCall {
    pub from: DeviceInfo,
    pub bytes: u64,
    pub files: u32,
    /// The apps it is about to send, valid ones, at most 256.
    pub apps: Vec<AppProbe>,
    reply: oneshot::Sender<PreflightAnswer>,
}

impl PreflightCall {
    pub(crate) fn new(from: DeviceInfo, bytes: u64, files: u32, apps: Vec<AppProbe>) -> (Self, oneshot::Receiver<PreflightAnswer>) {
        let (reply, rx) = oneshot::channel();
        (Self { from, bytes, files, apps, reply }, rx)
    }

    pub fn answer(self, answer: PreflightAnswer) {
        let _ = self.reply.send(answer);
    }
}

impl Node {
    /// Ask a device before sending it `bytes` in `files` files and perhaps these apps. An
    /// older device that doesn't answer preflights gives [`Error::Unsupported`]: send
    /// anyway, as before.
    pub async fn preflight(&self, to: &Fingerprint, bytes: u64, files: u32, apps: Vec<AppProbe>) -> Result<PreflightReply> {
        let peer = self.reach(to).await?;
        if !peer.features.iter().any(|f| f == "preflight") {
            return Err(Error::Unsupported);
        }
        let apps: Vec<AppProbe> = apps.into_iter().filter(AppProbe::is_valid).take(MAX_PROBES).collect();
        let (mut send, mut recv) = self.open_stream(to).await?;
        let sent = apps.len();
        write_msg(&mut send, &PreflightMsg::Preflight { bytes, files, apps }).await?;
        let _ = send.finish();
        match tokio::time::timeout(ASK_TIMEOUT, read_msg::<PreflightMsg>(&mut recv)).await.map_err(|_| Error::Timeout)?? {
            PreflightMsg::PreflightReply(reply) if reply.apps.len() <= sent => Ok(reply),
            PreflightMsg::PreflightReply(_) => Err(protocol("more apps than asked about")),
            PreflightMsg::Preflight { .. } => Err(protocol("expected an answer")),
        }
    }
}
