//! A phone's photos for the apps' Gallery tab (docs/protocol.md §4.18): pages passed
//! through, each thumbnail fetched once, a few at a time per phone, and kept in the
//! runtime folder (gone at logout) while there's room.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use linksy_core::gallery::GalleryAnswer;
use linksy_core::{Fingerprint, GalleryAlbum, GalleryCursor, GalleryItem, GalleryRequest, Node, Uuid};
use tokio::sync::{Semaphore, oneshot};

/// Thumbnails asked of one phone at once, and kept on disk at most.
const AT_ONCE: usize = 6;
const KEPT: usize = 3000;
pub const THUMB_SIZE: u32 = 256;

pub static GALLERY: OnceLock<Gallery> = OnceLock::new();

type Waiters = Vec<oneshot::Sender<Result<PathBuf, String>>>;

pub struct Gallery {
    dir: PathBuf,
    fetching: Mutex<HashMap<(Fingerprint, String), Waiters>>,
    slots: Mutex<HashMap<Fingerprint, Arc<Semaphore>>>,
    written: AtomicUsize,
}

impl Gallery {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, fetching: Mutex::default(), slots: Mutex::default(), written: AtomicUsize::new(0) }
    }

    pub async fn albums(&self, node: &Node, phone: &Fingerprint) -> Result<Vec<GalleryAlbum>, String> {
        match node.gallery(phone, GalleryRequest::Albums).await.map_err(|e| e.to_string())? {
            GalleryAnswer::Albums(albums) => Ok(albums),
            _ => Err("the phone answered something else".into()),
        }
    }

    pub async fn items(&self, node: &Node, phone: &Fingerprint, album: Option<String>, before: Option<GalleryCursor>, limit: u32) -> Result<(Vec<GalleryItem>, bool), String> {
        match node.gallery(phone, GalleryRequest::Items { album, before, limit }).await.map_err(|e| e.to_string())? {
            GalleryAnswer::Items { items, more } => Ok((items, more)),
            _ => Err("the phone answered something else".into()),
        }
    }

    /// Where item `id`'s thumbnail is, fetched first if it isn't there yet. Asked twice at
    /// once, it's fetched once.
    pub async fn thumb(&self, node: &Node, phone: &Fingerprint, id: &str) -> Result<PathBuf, String> {
        let path = self.path(phone, id);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(path);
        }
        let key = (*phone, id.to_string());
        let waiting = {
            let mut fetching = self.fetching.lock().unwrap();
            match fetching.get_mut(&key) {
                Some(waiters) => {
                    let (tx, rx) = oneshot::channel();
                    waiters.push(tx);
                    Some(rx)
                }
                None => {
                    fetching.insert(key.clone(), Vec::new());
                    None
                }
            }
        };
        if let Some(rx) = waiting {
            return rx.await.unwrap_or_else(|_| Err("cancelled".into()));
        }
        let result = self.fetch(node, phone, id, &path).await;
        let waiters = self.fetching.lock().unwrap().remove(&key).unwrap_or_default();
        for w in waiters {
            let _ = w.send(result.clone());
        }
        result
    }

    async fn fetch(&self, node: &Node, phone: &Fingerprint, id: &str, path: &Path) -> Result<PathBuf, String> {
        let slots = self.slots.lock().unwrap().entry(*phone).or_insert_with(|| Arc::new(Semaphore::new(AT_ONCE))).clone();
        let _slot = slots.acquire_owned().await.map_err(|e| e.to_string())?;
        let bytes = match node.gallery(phone, GalleryRequest::Thumb { id: id.into(), size: THUMB_SIZE }).await.map_err(|e| e.to_string())? {
            GalleryAnswer::Thumb(bytes) => bytes,
            _ => return Err("the phone answered something else".into()),
        };
        if !bytes.starts_with(&[0xff, 0xd8]) && !bytes.starts_with(b"\x89PNG") {
            return Err("the phone's thumbnail isn't a picture".into());
        }
        let dir = path.parent().ok_or("no folder for thumbnails")?;
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        let part = path.with_extension("part");
        tokio::fs::write(&part, &bytes).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&part, path).await.map_err(|e| e.to_string())?;
        if self.written.fetch_add(1, Ordering::Relaxed) % 200 == 199 {
            let root = self.dir.clone();
            tokio::task::spawn_blocking(move || prune(&root, KEPT));
        }
        Ok(path.to_path_buf())
    }

    /// Have the phone send these here, into the photos folder, as a transfer.
    pub async fn save(&self, node: &Node, phone: &Fingerprint, ids: Vec<String>, into: PathBuf) -> Result<Uuid, String> {
        node.gallery_send(phone, ids, into).await.map_err(|e| e.to_string())
    }

    fn path(&self, phone: &Fingerprint, id: &str) -> PathBuf {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut h);
        self.dir.join(phone.short()).join(format!("{:016x}.jpg", h.finish()))
    }
}

/// Keep the `keep` newest thumbnails, across phones.
fn prune(root: &Path, keep: usize) {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|phone| std::fs::read_dir(phone.path()).into_iter().flatten().flatten())
        .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f.path())))
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let phone = dir.path().join("A1B2");
        std::fs::create_dir_all(&phone).unwrap();
        for i in 0..5 {
            let f = phone.join(format!("{i}.jpg"));
            std::fs::write(&f, b"x").unwrap();
            let at = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i);
            std::fs::File::options().write(true).open(&f).unwrap().set_modified(at).unwrap();
        }
        prune(dir.path(), 2);
        let mut left: Vec<_> = std::fs::read_dir(&phone).unwrap().map(|f| f.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["3.jpg", "4.jpg"]);
    }
}
