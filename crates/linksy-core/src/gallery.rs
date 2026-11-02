//! A phone's photos and videos, browsed from a computer (docs/protocol.md §4.18), one
//! request per bidirectional stream (see [`GalleryRequest`]). The computer calls
//! [`Node::gallery`]; the phone takes requests from [`Node::incoming_gallery`] and answers
//! each [`GalleryCall`].

use std::path::PathBuf;

use crate::error::{Error, Result, invalid, protocol};
use crate::identity::{DeviceInfo, Fingerprint};
use crate::node::Node;
use crate::protocol::{GalleryAlbum, GalleryItem, GalleryReply, GalleryRequest, read_msg, write_msg};

/// Largest thumbnail, and most items in a page.
pub const MAX_THUMB: u64 = 256 << 10;
pub const MAX_ITEMS: u32 = 500;

#[derive(Debug)]
pub enum GalleryAnswer {
    Albums(Vec<GalleryAlbum>),
    Items { items: Vec<GalleryItem>, more: bool },
    Thumb(Vec<u8>),
    Ok,
}

/// A gallery request from a computer, to answer exactly once.
pub struct GalleryCall {
    pub from: DeviceInfo,
    pub request: GalleryRequest,
    send: quinn::SendStream,
}

impl GalleryCall {
    pub(crate) fn new(from: DeviceInfo, request: GalleryRequest, send: quinn::SendStream) -> Self {
        Self { from, request, send }
    }

    pub async fn albums(self, albums: Vec<GalleryAlbum>) -> Result<()> {
        self.reply(&GalleryReply::GalleryAlbums { albums }, &[]).await
    }

    pub async fn items(self, items: Vec<GalleryItem>, more: bool) -> Result<()> {
        self.reply(&GalleryReply::GalleryItems { items, more }, &[]).await
    }

    /// A JPEG, at most [`MAX_THUMB`] bytes.
    pub async fn thumb(self, jpeg: &[u8]) -> Result<()> {
        if jpeg.len() as u64 > MAX_THUMB {
            return self.error("thumbnail too large").await;
        }
        self.reply(&GalleryReply::GalleryData { length: jpeg.len() as u64 }, jpeg).await
    }

    pub async fn ok(self) -> Result<()> {
        self.reply(&GalleryReply::GalleryOk, &[]).await
    }

    pub async fn error(self, message: &str) -> Result<()> {
        self.reply(&GalleryReply::GalleryError { message: message.into() }, &[]).await
    }

    async fn reply(mut self, reply: &GalleryReply, bytes: &[u8]) -> Result<()> {
        write_msg(&mut self.send, reply).await?;
        self.send.write_all(bytes).await.map_err(|e| protocol(e.to_string()))?;
        self.send.finish().map_err(|e| protocol(e.to_string()))?;
        Ok(())
    }
}

impl Node {
    /// Ask a phone about its photos.
    pub async fn gallery(&self, to: &Fingerprint, request: GalleryRequest) -> Result<GalleryAnswer> {
        let request = match request {
            GalleryRequest::Items { album, before, limit } => GalleryRequest::Items { album, before, limit: limit.clamp(1, MAX_ITEMS) },
            GalleryRequest::Thumb { id, size } => GalleryRequest::Thumb { id, size: size.clamp(32, 1024) },
            other => other,
        };
        let (mut send, mut recv) = self.open_stream(to).await?;
        let mut first = serde_json::to_value(&request)?;
        first["type"] = "gallery".into();
        write_msg(&mut send, &first).await?;
        let _ = send.finish();
        match read_msg::<GalleryReply>(&mut recv).await? {
            GalleryReply::GalleryAlbums { albums } => Ok(GalleryAnswer::Albums(albums)),
            GalleryReply::GalleryItems { items, more } if items.len() <= MAX_ITEMS as usize => Ok(GalleryAnswer::Items { items, more }),
            GalleryReply::GalleryItems { .. } => Err(protocol("too many items at once")),
            GalleryReply::GalleryData { length } if length <= MAX_THUMB => {
                let mut data = vec![0u8; length as usize];
                recv.read_exact(&mut data).await.map_err(|e| protocol(e.to_string()))?;
                Ok(GalleryAnswer::Thumb(data))
            }
            GalleryReply::GalleryData { .. } => Err(protocol("thumbnail too large")),
            GalleryReply::GalleryOk => Ok(GalleryAnswer::Ok),
            GalleryReply::GalleryError { message } => Err(Error::Invalid(message)),
        }
    }

    /// Have a phone send these photos and videos here: they arrive as a transfer, taken
    /// without asking into `into`. Returns the transfer's id.
    pub async fn gallery_send(&self, to: &Fingerprint, ids: Vec<String>, into: PathBuf) -> Result<uuid::Uuid> {
        if ids.is_empty() {
            return Err(invalid("nothing to send"));
        }
        let transfer = uuid::Uuid::new_v4();
        self.expect_transfer(transfer, *to, into);
        match self.gallery(to, GalleryRequest::Send { ids, transfer }).await {
            Ok(_) => Ok(transfer),
            Err(e) => {
                self.unexpect_transfer(&transfer);
                Err(e)
            }
        }
    }
}
