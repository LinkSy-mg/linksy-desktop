//! What a phone's deck switches on this computer besides media, sound and locking
//! (docs/protocol.md §4.19): restarting and turning it off, Do Not Disturb. Each goes
//! through what this computer has (from the Flatpak, its own tools), and isn't offered
//! without it. Tools run in English and are stopped after 5 s.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use linksy_i18n::t;
use tracing::{debug, warn};

use crate::volume::output;

/// Longest a call may take.
const WAIT: Duration = Duration::from_secs(5);
const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
/// GNOME's Do Not Disturb is banners off.
const BANNERS: (&str, &str) = ("org.gnome.desktop.notifications", "show-banners");

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dnd {
    Gnome,
    /// Plasma's notifications, inhibited while the service holds a cookie.
    Kde,
}

/// What this computer has for each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// logind, to restart and turn off.
    pub power_off: bool,
    dnd: Option<Dnd>,
}

impl Found {
    pub fn dnd(&self) -> bool {
        self.dnd.is_some()
    }
}

#[derive(Default)]
pub struct Controls {
    found: OnceLock<Found>,
    /// Plasma's Do Not Disturb while on: the connection that asked (it ends with it) and
    /// the cookie.
    inhibit: tokio::sync::Mutex<Option<(zbus::Connection, u32)>>,
}

/// Which desktop this is: GNOME, KDE.
fn desktop() -> (bool, bool) {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    (desktop.contains("gnome"), desktop.contains("kde"))
}

impl Controls {
    /// Find out what's here, once.
    pub async fn probe(&self) -> Found {
        if let Some(found) = self.found.get() {
            return found.clone();
        }
        let (gnome, kde) = desktop();
        let gsettings = gnome && linksy_host::installed("gsettings");
        let power_off = linksy_host::exists(Path::new("/run/systemd/seats")) && (!linksy_host::flatpak() || linksy_host::installed("busctl"));
        let dnd = if gsettings && gsetting(BANNERS).await.is_some() {
            Some(Dnd::Gnome)
        } else if kde && inhibits().await {
            Some(Dnd::Kde)
        } else {
            None
        };
        self.found.get_or_init(|| Found { power_off, dnd }).clone()
    }

    fn found(&self) -> Found {
        self.found.get().cloned().unwrap_or_default()
    }

    /// Whether Do Not Disturb is on.
    pub async fn dnd(&self) -> Option<bool> {
        match self.found().dnd? {
            Dnd::Gnome => gsetting(BANNERS).await.map(|banners| !banners),
            Dnd::Kde => Some(self.inhibit.lock().await.is_some()),
        }
    }

    pub async fn toggle_dnd(&self) {
        match self.found().dnd {
            Some(Dnd::Gnome) => {
                if let Some(banners) = gsetting(BANNERS).await {
                    set_gsetting(BANNERS, !banners).await;
                }
            }
            Some(Dnd::Kde) => {
                let mut held = self.inhibit.lock().await;
                let done = match held.take() {
                    Some((bus, cookie)) => tokio::time::timeout(WAIT, bus.call_method(Some(NOTIFICATIONS), NOTIFICATIONS_PATH, Some(NOTIFICATIONS), "UnInhibit", &(cookie,))).await.map(|r| r.map(|_| ())),
                    None => tokio::time::timeout(WAIT, inhibit()).await.map(|r| r.map(|on| *held = Some(on))),
                };
                match done {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => debug!("deck: Do Not Disturb: {e}"),
                    Err(_) => debug!("deck: Do Not Disturb: no answer"),
                }
            }
            None => {}
        }
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

/// Whether the notification server can be inhibited (Plasma's can).
async fn inhibits() -> bool {
    let asked = async {
        let bus = zbus::Connection::session().await?;
        let reply = bus.call_method(Some(NOTIFICATIONS), NOTIFICATIONS_PATH, Some(NOTIFICATIONS), "GetCapabilities", &()).await?;
        reply.body().deserialize::<Vec<String>>()
    };
    matches!(tokio::time::timeout(WAIT, asked).await, Ok(Ok(caps)) if caps.iter().any(|c| c == "inhibitions"))
}

/// Hold the notifications back on a connection of its own, kept while it's on.
async fn inhibit() -> zbus::Result<(zbus::Connection, u32)> {
    let bus = zbus::Connection::session().await?;
    let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    let reply = bus.call_method(Some(NOTIFICATIONS), NOTIFICATIONS_PATH, Some(NOTIFICATIONS), "Inhibit", &("mg.diary.linksy.Linksy", t!("Turned on from a phone's deck"), hints)).await?;
    let cookie: u32 = reply.body().deserialize()?;
    Ok((bus, cookie))
}

/// A boolean setting, from `gsettings get` (`true` or `false`).
async fn gsetting((schema, key): (&str, &str)) -> Option<bool> {
    parse_bool(&output("gsettings", &["get", schema, key]).await?)
}

async fn set_gsetting((schema, key): (&str, &str), on: bool) {
    if output("gsettings", &["set", schema, key, if on { "true" } else { "false" }]).await.is_none() {
        debug!("deck: gsettings set {schema} {key} failed");
    }
}

fn parse_bool(text: &str) -> Option<bool> {
    match text.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_the_tools_print() {
        assert_eq!(parse_bool("true\n"), Some(true));
        assert_eq!(parse_bool(" false "), Some(false));
        assert_eq!(parse_bool("No such key “show-banners”"), None);
    }
}
