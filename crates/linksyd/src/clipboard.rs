//! Universal clipboard: text shared by a paired device lands on this clipboard right away.
//! Wayland only lets the focused app set the clipboard, so the copy goes through the
//! desktop's clipboard tool (wl-copy, xclip or xsel), which can.

use std::io::Write;
use std::process::{Command, Stdio};

use linksy_core::{Event, Node};
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, info};

pub async fn run(node: Node, notifications: bool) {
    let mut events = node.subscribe();
    loop {
        let (from, text) = match events.recv().await {
            Ok(Event::Clipboard { from, text }) => (from.name, text),
            Ok(_) | Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
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
            #[cfg(not(target_os = "linux"))]
            let _ = notifications;
        });
    }
}

/// "Copied from X", or, when no clipboard tool worked, a Copy button to try again.
#[cfg(target_os = "linux")]
fn announce(from: &str, text: &str, copied: bool) {
    let mut preview: String = text.chars().take(120).collect();
    if preview.len() < text.len() {
        preview = format!("{}…", preview.trim_end());
    }
    let summary = if copied { format!("Copied from {from}") } else { format!("Clipboard from {from}") };
    let mut n = crate::notify::base(&summary, &preview);
    if copied {
        let _ = n.show();
        return;
    }
    n.action("default", "Copy").action("copy", "Copy");
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
