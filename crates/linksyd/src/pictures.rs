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
/// The biggest file looked at when finding this computer's own picture.
const MAX_FILE: usize = 8 * 1024 * 1024;

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

/// The user's account picture (GNOME's Users settings, else ~/.face), in base64, as a JPEG or
/// PNG that fits what a picture may weigh. Read on the computer itself: the Flatpak's
/// sandbox doesn't have those folders.
fn own_picture() -> Option<String> {
    let user = std::env::var("USER").ok();
    let candidates = user.map(|u| PathBuf::from("/var/lib/AccountsService/icons").join(u)).into_iter().chain(std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".face")));
    let bytes = candidates.filter_map(|p| linksy_host::read_bytes(&p, MAX_FILE)).filter(|b| is_image(b)).find_map(fit)?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// The picture as it is when it's light enough, else scaled down and saved as JPEG: account
/// pictures are often photos of a few hundred kilobytes.
fn fit(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.len() <= MAX {
        return Some(bytes);
    }
    let picture = image::load_from_memory(&bytes).ok()?;
    for side in [512, 384, 256, 128] {
        let mut out = Vec::new();
        let rgb = picture.thumbnail(side, side).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&rgb).ok()?;
        if out.len() <= MAX {
            return Some(out);
        }
    }
    None
}

fn is_image(b: &[u8]) -> bool {
    b.starts_with(b"\x89PNG\r\n\x1a\n") || b.starts_with(&[0xff, 0xd8, 0xff])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big_png() -> Vec<u8> {
        let img = image::RgbImage::from_fn(700, 700, |x, y| image::Rgb([((x * 7) ^ (y * 13)) as u8, (x * 3 + y * 5) as u8, ((x * y) % 251) as u8]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_light_picture_is_kept_as_it_is() {
        let small = b"\x89PNG\r\n\x1a\nnot really a picture".to_vec();
        assert_eq!(fit(small.clone()), Some(small));
    }

    #[test]
    fn a_heavy_picture_is_scaled_down_to_fit() {
        let png = big_png();
        assert!(png.len() > MAX, "the test picture is {} bytes, it must be heavier than {MAX}", png.len());
        let fitted = fit(png).expect("scaled down");
        assert!(fitted.len() <= MAX);
        assert!(is_image(&fitted));
    }

    #[test]
    fn something_that_is_no_picture_is_left_out() {
        let mut junk = vec![0xff, 0xd8, 0xff];
        junk.resize(MAX + 10, 1);
        assert_eq!(fit(junk), None);
    }
}
