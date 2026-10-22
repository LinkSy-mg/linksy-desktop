//! Universal clipboard: text, images and files shared by a paired device land on this
//! clipboard right away. Wayland only lets the focused app set the clipboard, so the copy
//! goes through the desktop's clipboard tool (wl-copy, xclip or xsel), which can.
//!
//! Devices can also watch this clipboard (`clipboard_watch`): every new text copied here
//! goes to them. That needs `wl-paste --watch`, which only works where the compositor has
//! the data-control protocol (KDE, wlroots), not on GNOME.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use linksy_core::{Event, Fingerprint, Node};
use linksy_i18n::t;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub async fn run(node: Node, notifications: bool) {
    let mut events = node.subscribe();
    let mut watchers: HashSet<Fingerprint> = HashSet::new();
    let mut changes: Option<mpsc::UnboundedReceiver<String>> = None;
    // The text last on the clipboard, so what a device sent isn't sent straight back.
    let mut last = String::new();
    loop {
        let event = tokio::select! {
            e = events.recv() => e,
            Some(text) = async { changes.as_mut()?.recv().await } => {
                if text != last {
                    last = text;
                    for to in &watchers {
                        if let Err(e) = node.send_clipboard(to, &last).await {
                            debug!("clipboard to {to} failed: {e}");
                        }
                    }
                }
                continue;
            }
        };
        match event {
            Ok(Event::Clipboard { from, text }) => {
                last = text.clone();
                let from = from.name;
                tokio::task::spawn_blocking(move || {
                    let copied = copy_text(&text);
                    match &copied {
                        Ok(()) => info!("copied clipboard from {from}"),
                        Err(e) => debug!("copy failed: {e}"),
                    }
                    #[cfg(target_os = "linux")]
                    if notifications {
                        announce(&from, &text, copied.is_ok());
                    }
                });
            }
            Ok(Event::ClipboardFile { from, path }) => {
                let from = from.name;
                tokio::task::spawn_blocking(move || {
                    let copied = copy_file(&path);
                    match &copied {
                        Ok(()) => info!("copied {} from {from}", path.display()),
                        Err(e) => warn!("copying {} failed: {e}", path.display()),
                    }
                    #[cfg(target_os = "linux")]
                    if notifications {
                        let body = match (copied.is_ok(), image_type(&path)) {
                            (true, Some(_)) => t!("Image ready to paste").to_string(),
                            (true, None) => t!("{name} ready to paste", name = file_name(&path)),
                            (false, _) => t!("Couldn't copy {name}", name = file_name(&path)),
                        };
                        let _ = crate::notify::base(&t!("Copied from {device}", device = from), &body).show();
                    }
                });
            }
            Ok(Event::ClipboardWatch { from, on }) => {
                if !on {
                    watchers.remove(&from.id);
                    continue;
                }
                if changes.is_none() && can_watch() {
                    changes = watch();
                }
                if changes.is_some() {
                    info!("{} watches the clipboard", from.name);
                    watchers.insert(from.id);
                }
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
        #[cfg(not(target_os = "linux"))]
        let _ = notifications;
    }
}

/// Whether this desktop lets Linksy see clipboard changes without being focused.
pub fn can_watch() -> bool {
    static CAN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CAN.get_or_init(probe_watch)
}

fn probe_watch() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return false;
    }
    let Ok(mut child) = Command::new("wl-paste").args(["--watch", "true"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    // Unsupported, it quits at once; supported, it keeps running.
    // Schrödinger's clipboard: we only know by waiting.
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    true
}

/// Every new text on the clipboard, from `wl-paste --watch`. None where it can't watch.
fn watch() -> Option<mpsc::UnboundedReceiver<String>> {
    // Each change prints the text and a NUL.
    let mut child = Command::new("wl-paste")
        .args(["--type", "text", "--watch", "sh", "-c", "cat; printf '\\0'"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| debug!("wl-paste: {e}"))
        .ok()?;
    let out = child.stdout.take()?;
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let mut out = BufReader::new(out);
        let mut buf = Vec::new();
        while matches!(out.read_until(0, &mut buf), Ok(n) if n > 0) {
            if buf.last() == Some(&0) {
                buf.pop();
            }
            if let Ok(text) = String::from_utf8(std::mem::take(&mut buf))
                && !text.is_empty()
                && tx.send(text).is_err()
            {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        debug!("clipboard watch ended");
    });
    Some(rx)
}

/// "Copied from X", or, when no clipboard tool worked, a Copy button to try again.
#[cfg(target_os = "linux")]
fn announce(from: &str, text: &str, copied: bool) {
    let mut preview: String = text.chars().take(120).collect();
    if preview.len() < text.len() {
        preview = format!("{}…", preview.trim_end());
    }
    let summary = if copied { t!("Copied from {device}", device = from) } else { t!("Clipboard from {device}", device = from) };
    let mut n = crate::notify::base(&summary, &preview);
    if copied {
        let _ = n.show();
        return;
    }
    n.action("default", t!("Copy")).action("copy", t!("Copy"));
    if let Ok(handle) = n.show() {
        handle.wait_for_action(|action| {
            if matches!(action, "default" | "copy") {
                let _ = copy_text(text);
            }
        });
    }
}

fn copy_text(text: &str) -> std::io::Result<()> {
    let tools: &[(&str, &[&str])] = &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])];
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no clipboard tool (wl-copy, xclip or xsel)");
    for (cmd, args) in tools {
        let child = Command::new(cmd).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(mut child) => {
                child.stdin.take().expect("piped stdin").write_all(text.as_bytes())?;
                return match child.wait()? {
                    s if s.success() => Ok(()),
                    s => Err(std::io::Error::other(format!("{cmd} exited with {s}"))),
                };
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// An image goes on the clipboard as one, so it pastes into a chat or editor; anything else
/// as a file, so it pastes into the file manager.
fn copy_file(path: &Path) -> std::io::Result<()> {
    match image_type(path) {
        Some(mime) => copy_as(mime, &std::fs::read(path)?),
        None => copy_as("text/uri-list", format!("{}\r\n", file_uri(path)).as_bytes()),
    }
}

fn copy_as(mime: &str, data: &[u8]) -> std::io::Result<()> {
    let tools: &[(&str, &[&str])] = &[("wl-copy", &["--type", mime]), ("xclip", &["-selection", "clipboard", "-t", mime])];
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no clipboard tool (wl-copy or xclip)");
    for (cmd, args) in tools {
        let child = Command::new(cmd).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(mut child) => {
                child.stdin.take().expect("piped stdin").write_all(data)?;
                return match child.wait()? {
                    s if s.success() => Ok(()),
                    s => Err(std::io::Error::other(format!("{cmd} exited with {s}"))),
                };
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn image_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `file://` URI with everything but unreserved characters and `/` percent-encoded.
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_are_encoded() {
        assert_eq!(file_uri(Path::new("/home/a/My File#1.pdf")), "file:///home/a/My%20File%231.pdf");
        assert_eq!(image_type(Path::new("x.PNG")), Some("image/png"));
        assert_eq!(image_type(Path::new("x.pdf")), None);
    }
}
