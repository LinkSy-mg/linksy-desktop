//! "Open on PC": web links a paired device shares open in this computer's browser.

use std::process::{Command, Stdio};

use linksy_core::{Event, Node};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

pub async fn run(node: Node) {
    let mut events = node.subscribe();
    loop {
        let (from, url) = match events.recv().await {
            Ok(Event::OpenUrl { from, url }) => (from.name, url),
            Ok(_) | Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
        // The core only lets http(s) links through, so this never opens a file or a program.
        // In a Flatpak, xdg-open hands the link to the OpenURI portal.
        match Command::new("xdg-open").arg(&url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(mut child) => {
                info!("opening a link from {from}");
                std::thread::spawn(move || child.wait());
            }
            Err(e) => warn!("could not open a link from {from}: {e}"),
        }
    }
}
