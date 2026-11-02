//! What a phone's deck switches on this computer besides media, sound and locking
//! (docs/protocol.md §4.19): restarting and turning it off. Each goes through what this
//! computer has (from the Flatpak, its own tools), and isn't offered without it. Tools run
//! in English and are stopped after 5 s.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use tracing::warn;

use crate::volume::output;

/// Longest a call may take.
const WAIT: Duration = Duration::from_secs(5);
const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";

/// What this computer has for each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// logind, to restart and turn off.
    pub power_off: bool,
}

#[derive(Default)]
pub struct Controls {
    found: OnceLock<Found>,
}

impl Controls {
    /// Find out what's here, once.
    pub async fn probe(&self) -> Found {
        if let Some(found) = self.found.get() {
            return found.clone();
        }
        let power_off = linksy_host::exists(Path::new("/run/systemd/seats")) && (!linksy_host::flatpak() || linksy_host::installed("busctl"));
        self.found.get_or_init(|| Found { power_off }).clone()
    }

    /// Restart or turn off through logind, which asks for a password where its rules want one.
    pub async fn power_off(&self, restart: bool) {
        let method = if restart { "Reboot" } else { "PowerOff" };
        let asked = async {
            let bus = zbus::Connection::system().await?;
            bus.call_method(Some(LOGIN1), LOGIN1_PATH, Some(LOGIN1_MANAGER), method, &(true,)).await.map(|_| ())
        };
        let failed = match tokio::time::timeout(WAIT, asked).await {
            Ok(Ok(())) => return,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "no answer (a password asked on the computer?)".to_string(),
        };
        // The Flatpak's system bus doesn't reach logind; the computer's busctl does
        if linksy_host::flatpak() && output("busctl", &["call", "--system", LOGIN1, LOGIN1_PATH, LOGIN1_MANAGER, method, "b", "true"]).await.is_some() {
            return;
        }
        warn!("deck: could not {}: {failed}", if restart { "restart" } else { "turn off" });
    }
}
