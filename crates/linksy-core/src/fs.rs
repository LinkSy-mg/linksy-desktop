//! Browsing another device's files, one request per bidirectional stream (see
//! [`FsRequest`]). The asking side calls [`Node::fs`]; the serving side (a phone) takes
//! requests from [`Node::incoming_fs`] and answers each [`FsCall`].

use crate::error::{Error, Result, protocol};
use crate::identity::{DeviceInfo, Fingerprint};
use crate::node::Node;
use crate::protocol::{FsEntry, FsReply, FsRequest, read_msg, write_msg};

/// Largest single read or write; bigger files go in several.
pub const MAX_CHUNK: u64 = 4 << 20;

/// What a request got back.
#[derive(Debug)]
pub enum FsAnswer {
    Entries(Vec<FsEntry>),
    Entry(FsEntry),
    Data(Vec<u8>),
    Ok,
}

/// A file request from another device, to answer exactly once.
pub struct FsCall {
    pub from: DeviceInfo,
    pub request: FsRequest,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl FsCall {
    pub(crate) fn new(from: DeviceInfo, request: FsRequest, send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self { from, request, send, recv }
    }

    /// For `write`: the bytes to write (`length` of them).
    pub async fn body(&mut self) -> Result<Vec<u8>> {
        let FsRequest::Write { length, .. } = self.request else { return Ok(Vec::new()) };
        let mut buf = vec![0u8; length.min(MAX_CHUNK) as usize];
        self.recv.read_exact(&mut buf).await.map_err(|e| protocol(e.to_string()))?;
        Ok(buf)
    }

    pub async fn entries(self, entries: Vec<FsEntry>) -> Result<()> {
        self.reply(&FsReply::FsEntries { entries }, &[]).await
    }

    pub async fn entry(self, entry: FsEntry) -> Result<()> {
        self.reply(&FsReply::FsEntry { entry }, &[]).await
    }

    /// For `read`: the bytes from the offset (at most the length asked for).
    pub async fn data(self, bytes: &[u8]) -> Result<()> {
        self.reply(&FsReply::FsData { length: bytes.len() as u64 }, bytes).await
    }

    pub async fn ok(self) -> Result<()> {
        self.reply(&FsReply::FsOk, &[]).await
    }

    pub async fn error(self, message: &str, not_found: bool) -> Result<()> {
        self.reply(&FsReply::FsError { message: message.into(), not_found }, &[]).await
    }

    async fn reply(mut self, reply: &FsReply, bytes: &[u8]) -> Result<()> {
        write_msg(&mut self.send, reply).await?;
        self.send.write_all(bytes).await.map_err(|e| protocol(e.to_string()))?;
        self.send.finish().map_err(|e| protocol(e.to_string()))?;
        Ok(())
    }
}

impl Node {
    /// Ask a device about its files. `body` goes with a `write` (its `length` bytes).
    pub async fn fs(&self, to: &Fingerprint, request: FsRequest, body: &[u8]) -> Result<FsAnswer> {
        if let FsRequest::Read { length, .. } | FsRequest::Write { length, .. } = request
            && length > MAX_CHUNK
        {
            return Err(crate::error::invalid("at most 4 MiB at a time"));
        }
        if let FsRequest::Write { length, .. } = request
            && length != body.len() as u64
        {
            return Err(crate::error::invalid("write length doesn't match the data"));
        }
        let (mut send, mut recv) = self.open_stream(to).await?;
        let mut first = serde_json::to_value(&request)?;
        first["type"] = "fs".into();
        write_msg(&mut send, &first).await?;
        send.write_all(body).await.map_err(|e| protocol(e.to_string()))?;
        let _ = send.finish();
        match read_msg::<FsReply>(&mut recv).await? {
            FsReply::FsEntries { entries } => Ok(FsAnswer::Entries(entries)),
            FsReply::FsEntry { entry } => Ok(FsAnswer::Entry(entry)),
            FsReply::FsData { length } if length <= MAX_CHUNK => {
                let mut data = vec![0u8; length as usize];
                recv.read_exact(&mut data).await.map_err(|e| protocol(e.to_string()))?;
                Ok(FsAnswer::Data(data))
            }
            FsReply::FsData { .. } => Err(protocol("too much data at once")),
            FsReply::FsOk => Ok(FsAnswer::Ok),
            FsReply::FsError { message, not_found: true } => Err(Error::NotFound(message)),
            FsReply::FsError { message, .. } => Err(Error::Invalid(message)),
        }
    }
}
