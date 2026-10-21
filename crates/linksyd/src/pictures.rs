//! Device pictures (docs/android-app.md §2.7): each device sends its own picture after connecting,
//! and the other shows it as that device's avatar. This computer's is the user's account
//! picture; received ones are kept in the data dir, one file per device.

use std::path::PathBuf;
use std::sync::OnceLock;

use base64::Engine;
use linksy_core::{Event, Fingerprint, Node};
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// What a picture may weigh (the protocol's limit).
const MAX: usize = 128 * 1024;

/// The picture `id` sent, if it sent one.
pub fn path(id: &Fingerprint) -> Option<PathBuf> {
    let p = DIR.get()?.join(format!("{id}.img"));
    p.is_file().then_some(p)
}

pub async fn run(node: Node, data_dir: PathBuf) {
    let dir = DIR.get_or_init(|| data_dir.join("pictures")).clone();
    let mut events = node.subscribe();
    loop {
        match events.recv().await {
            Ok(Event::Connected { device }) => {
                let Some(image) = own_picture() else { continue };
                let node = node.clone();
                tokio::spawn(async move {
                    if let Err(e) = node.send_picture(&device.id, &image).await {
                        debug!("picture to {}: {e}", device.name);
                    }
                });
            }
            Ok(Event::Picture { from, image }) => {
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(image.trim()) else { continue };
                if bytes.len() > MAX || !is_image(&bytes) {
                    continue;
                }
                let path = dir.join(format!("{}.img", from.id));
                let saved = std::fs::create_dir_all(&dir).and_then(|()| {
                    let part = path.with_extension("part");
                    std::fs::write(&part, &bytes)?;
                    std::fs::rename(&part, &path)
                });
                if let Err(e) = saved {
                    warn!("could not save {}'s picture: {e}", from.name);
                }
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

/// The user's account picture (GNOME's Users settings, else ~/.face), in base64, when it's a
/// small enough JPEG or PNG.
fn own_picture() -> Option<String> {
    let user = std::env::var("USER").ok();
    let candidates = user.map(|u| PathBuf::from("/var/lib/AccountsService/icons").join(u)).into_iter().chain(std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".face")));
    let bytes = candidates.filter_map(|p| std::fs::read(p).ok()).find(|b| b.len() <= MAX && is_image(b))?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn is_image(b: &[u8]) -> bool {
    b.starts_with(b"\x89PNG\r\n\x1a\n") || b.starts_with(&[0xff, 0xd8, 0xff])
}
