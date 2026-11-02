//! What a phone's deck switches on this computer besides media, sound and locking
//! (docs/protocol.md §4.19): restarting and turning it off, Do Not Disturb, Wi-Fi,
//! Bluetooth, airplane mode and the power profile. Each goes through what this computer has
//! (from the Flatpak, its own tools), and isn't offered without it. Tools run in English
//! and are stopped after 5 s.

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
const LOW_BATTERY: (&str, &str) = ("org.gnome.settings-daemon.plugins.power", "power-saver-profile-on-low-battery");
/// The power profiles a deck sets, as powerprofilesctl names them.
pub const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

const RFKILL: &str = "org.gnome.SettingsDaemon.Rfkill";
const RFKILL_PATH: &str = "/org/gnome/SettingsDaemon/Rfkill";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dnd {
    Gnome,
    /// Plasma's notifications, inhibited while the service holds a cookie.
    Kde,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Bluetooth {
    Bluetoothctl,
    Rfkill,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Airplane {
    /// GNOME's own airplane mode.
    Gnome,
    /// Every radio off with nmcli, and Bluetooth.
    Radios,
}

/// What this computer has for each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// logind, to restart and turn off.
    pub power_off: bool,
    dnd: Option<Dnd>,
    /// nmcli.
    pub wifi: bool,
    bluetooth: Option<Bluetooth>,
    airplane: Option<Airplane>,
    /// The ones of [`PROFILES`] `powerprofilesctl list` has.
    pub profiles: Vec<String>,
    /// GNOME's power saver on low battery.
    pub power_auto: bool,
}

impl Found {
    pub fn dnd(&self) -> bool {
        self.dnd.is_some()
    }

    pub fn bluetooth(&self) -> bool {
        self.bluetooth.is_some()
    }

    pub fn airplane(&self) -> bool {
        self.airplane.is_some()
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
        let wifi = linksy_host::installed("nmcli");
        let bluetooth = if linksy_host::installed("bluetoothctl") {
            Some(Bluetooth::Bluetoothctl)
        } else if linksy_host::installed("rfkill") {
            Some(Bluetooth::Rfkill)
        } else {
            None
        };
        let airplane = if gnome && gnome_airplane().await.is_some() {
            Some(Airplane::Gnome)
        } else if wifi {
            Some(Airplane::Radios)
        } else {
            None
        };
        let profiles = match linksy_host::installed("powerprofilesctl") {
            true => output("powerprofilesctl", &["list"]).await.map(|o| parse_profiles(&o)).unwrap_or_default(),
            false => Vec::new(),
        };
        let power_auto = gsettings && gsetting(LOW_BATTERY).await.is_some();
        self.found.get_or_init(|| Found { power_off, dnd, wifi, bluetooth, airplane, profiles, power_auto }).clone()
    }

    /// The power profile now: one of [`PROFILES`], or another.
    pub async fn profile(&self) -> Option<String> {
        if self.found().profiles.is_empty() {
            return None;
        }
        let name = output("powerprofilesctl", &["get"]).await?.trim().to_string();
        (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some(name)
    }

    pub async fn set_profile(&self, profile: &str) {
        if self.found().profiles.iter().any(|p| p == profile) {
            run("powerprofilesctl", &["set", profile]).await;
        }
    }

    /// Whether GNOME turns the power saver on when the battery is low.
    pub async fn power_auto(&self) -> Option<bool> {
        if !self.found().power_auto {
            return None;
        }
        gsetting(LOW_BATTERY).await
    }

    pub async fn toggle_power_auto(&self) {
        if let Some(on) = self.power_auto().await {
            set_gsetting(LOW_BATTERY, !on).await;
        }
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

    /// Whether Wi-Fi is on.
    pub async fn wifi(&self) -> Option<bool> {
        if !self.found().wifi {
            return None;
        }
        radio(&output("nmcli", &["radio", "wifi"]).await?)
    }

    pub async fn toggle_wifi(&self) {
        if let Some(on) = self.wifi().await {
            run("nmcli", &["radio", "wifi", if on { "off" } else { "on" }]).await;
        }
    }

    /// Whether Bluetooth is on.
    pub async fn bluetooth(&self) -> Option<bool> {
        match self.found().bluetooth? {
            Bluetooth::Bluetoothctl => powered(&output("bluetoothctl", &["show"]).await?),
            Bluetooth::Rfkill => unblocked(&output("rfkill", &["list", "bluetooth"]).await?),
        }
    }

    async fn set_bluetooth(&self, on: bool) {
        match self.found().bluetooth {
            Some(Bluetooth::Bluetoothctl) => run("bluetoothctl", &["power", if on { "on" } else { "off" }]).await,
            Some(Bluetooth::Rfkill) => run("rfkill", &[if on { "unblock" } else { "block" }, "bluetooth"]).await,
            None => {}
        }
    }

    pub async fn toggle_bluetooth(&self) {
        if let Some(on) = self.bluetooth().await {
            self.set_bluetooth(!on).await;
        }
    }

    /// Whether airplane mode is on: elsewhere than GNOME, every radio off.
    pub async fn airplane(&self) -> Option<bool> {
        match self.found().airplane? {
            Airplane::Gnome => gnome_airplane().await,
            Airplane::Radios => {
                let (wifi, wwan, bluetooth) = tokio::join!(self.wifi(), async { radio(&output("nmcli", &["radio", "wwan"]).await?) }, self.bluetooth());
                Some(!wifi? && wwan != Some(true) && bluetooth != Some(true))
            }
        }
    }

    pub async fn toggle_airplane(&self) {
        let Some(on) = self.airplane().await else { return };
        match self.found().airplane {
            Some(Airplane::Gnome) => {
                let asked = async {
                    let bus = zbus::Connection::session().await?;
                    let value = zbus::zvariant::Value::from(!on);
                    bus.call_method(Some(RFKILL), RFKILL_PATH, Some("org.freedesktop.DBus.Properties"), "Set", &(RFKILL, "AirplaneMode", value)).await.map(|_| ())
                };
                if !matches!(tokio::time::timeout(WAIT, asked).await, Ok(Ok(()))) {
                    debug!("deck: GNOME didn't change airplane mode");
                }
            }
            Some(Airplane::Radios) => {
                run("nmcli", &["radio", "all", if on { "on" } else { "off" }]).await;
                self.set_bluetooth(on).await;
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

/// GNOME's airplane mode, where it has one.
async fn gnome_airplane() -> Option<bool> {
    let read = async {
        let bus = zbus::Connection::session().await?;
        let get = |name: &'static str| {
            let bus = bus.clone();
            async move {
                let reply = bus.call_method(Some(RFKILL), RFKILL_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(RFKILL, name)).await?;
                let value: zbus::zvariant::OwnedValue = reply.body().deserialize()?;
                bool::try_from(value).map_err(zbus::Error::from)
            }
        };
        Ok::<_, zbus::Error>((get("HasAirplaneMode").await?, get("AirplaneMode").await?))
    };
    match tokio::time::timeout(WAIT, read).await {
        Ok(Ok((true, on))) => Some(on),
        _ => None,
    }
}

/// Run a tool for what it does.
async fn run(program: &str, args: &[&str]) {
    if output(program, args).await.is_none() {
        debug!("deck: {program} {} failed", args.join(" "));
    }
}

/// `nmcli radio wifi`: `enabled` or `disabled`.
fn radio(text: &str) -> Option<bool> {
    match text.trim() {
        "enabled" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
}

/// `bluetoothctl show`: `Powered: yes` under the default controller.
fn powered(text: &str) -> Option<bool> {
    text.lines().find_map(|l| l.trim().strip_prefix("Powered:")).and_then(|v| match v.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    })
}

/// `rfkill list bluetooth`: on when one isn't blocked either way.
fn unblocked(text: &str) -> Option<bool> {
    let mut devices: Vec<(bool, bool)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if !line.contains("blocked") && line.contains(": ") {
            devices.push((false, false));
        } else if let (Some(d), Some(v)) = (devices.last_mut(), line.strip_prefix("Soft blocked:")) {
            d.0 = v.trim() == "no";
        } else if let (Some(d), Some(v)) = (devices.last_mut(), line.strip_prefix("Hard blocked:")) {
            d.1 = v.trim() == "no";
        }
    }
    (!devices.is_empty()).then(|| devices.iter().any(|&(soft, hard)| soft && hard))
}

/// `powerprofilesctl list`: each profile a line of its own (`* balanced:`, the active one
/// starred), its details indented under it.
fn parse_profiles(text: &str) -> Vec<String> {
    let listed: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("* ").or_else(|| l.strip_prefix("  ")))
        .filter(|l| !l.starts_with(' '))
        .filter_map(|l| l.trim_end().strip_suffix(':'))
        .collect();
    PROFILES.iter().filter(|p| listed.contains(p)).map(|p| p.to_string()).collect()
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
        assert_eq!((radio("enabled\n"), radio("disabled"), radio("Error: NetworkManager is not running.")), (Some(true), Some(false), None));
        let show = "Controller 00:11:22:33:44:55 (public)\n\tName: pc\n\tPowered: yes\n\tDiscoverable: no\n";
        assert_eq!(powered(show), Some(true));
        assert_eq!(powered(&show.replace("Powered: yes", "Powered: no")), Some(false));
        assert_eq!(powered("No default controller available\n"), None);
        let rfkill = "0: hci0: Bluetooth\n\tSoft blocked: yes\n\tHard blocked: no\n1: hci1: Bluetooth\n\tSoft blocked: no\n\tHard blocked: no\n";
        assert_eq!(unblocked(rfkill), Some(true));
        assert_eq!(unblocked(&rfkill.replace("Soft blocked: no", "Soft blocked: yes")), Some(false));
        assert_eq!(unblocked(""), None);
        let list = "  performance:\n    CpuDriver:\tamd_pstate\n    Degraded:   no\n\n* balanced:\n    CpuDriver:\tamd_pstate\n    PlatformDriver:\tplatform_profile\n\n  power-saver:\n    CpuDriver:\tamd_pstate\n";
        assert_eq!(parse_profiles(list), PROFILES);
        assert_eq!(parse_profiles("* balanced:\n    CpuDriver:\tintel_pstate\n\n  power-saver:\n"), ["power-saver", "balanced"]);
        assert!(parse_profiles("Failed to activate service 'net.hadess.PowerProfiles'").is_empty());
    }
}
