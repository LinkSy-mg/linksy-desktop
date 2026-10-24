//! What's New: the few things each version brings, shown once by both apps the first time
//! a new version opens, and from the menu any time. Not the changelog: the highlights a
//! person would notice, in their words, with a link to the full release notes. A fresh
//! install shows nothing (there's nothing new to someone who just arrived).
//!
//! Each release adds its highlights at the top of [`releases`] when it's cut.

use std::path::PathBuf;

use linksy_i18n::t;

/// Linksy's source, for "Star Linksy on GitHub".
pub const REPO: &str = "https://github.com/LinkSy-mg/linksy-desktop";

/// At most this many highlights at once, when several versions were skipped.
const MOST: usize = 5;

/// One thing worth noticing.
#[derive(Clone)]
pub struct Highlight {
    /// A symbolic icon: `gtk` from Adwaita's set, `qt` from Breeze's.
    pub gtk: &'static str,
    pub qt: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

struct Release {
    version: &'static str,
    highlights: Vec<Highlight>,
}

/// Newest first.
fn releases() -> Vec<Release> {
    vec![
        Release {
            version: "0.9.0",
            highlights: vec![
                Highlight {
                    gtk: "computer-symbolic",
                    qt: "computer",
                    title: t!("On More Computers"),
                    body: t!("Ubuntu 24.04 and Linux Mint 22, Hyprland, Sway, niri and X11 desktops: a second display on Hyprland and Sway, your screen locker for Lock When Away, and Finish Setup that knows what they lack."),
                },
                Highlight {
                    gtk: "network-wireless-symbolic",
                    qt: "network-wireless",
                    title: t!("Private on Public Wi-Fi"),
                    body: t!("Linksy no longer tells everyone on the network this computer's name. Your devices still find it; others see a code that changes every 15 minutes."),
                },
                Highlight {
                    gtk: "folder-download-symbolic",
                    qt: "folder-download",
                    title: t!("One Accept for a Batch"),
                    body: t!("Files sent together ask once, and Always Accept saves a device's files without asking."),
                },
            ],
        },
        Release {
            version: "0.8.7",
            highlights: vec![
                Highlight {
                    gtk: "media-removable-symbolic",
                    qt: "drive-removable-media-usb",
                    title: t!("Plug In to Charge, Nothing Asked"),
                    body: t!("A phone plugged in is left alone. Connect by USB Cable uses the cable when you want it, or turn on Use the Cable Automatically for a phone."),
                },
                Highlight {
                    gtk: "camera-web-symbolic",
                    qt: "camera-web",
                    title: t!("The Flatpak's Webcam in Every App"),
                    body: t!("In the Flatpak, Finish Setup gives the one command that adds the camera driver, and the USB guide the one for the cable."),
                },
            ],
        },
        Release {
            version: "0.8.6",
            highlights: vec![Highlight {
                gtk: "software-update-available-symbolic",
                qt: "update-none",
                title: t!("Update in One Click"),
                body: t!("When a new version is out, Update Now installs it, checked and with one password, and starts Linksy again. In a terminal: linksy upgrade."),
            }],
        },
        Release {
            version: "0.8.5",
            highlights: vec![
                Highlight {
                    gtk: "preferences-system-notifications-symbolic",
                    qt: "notifications",
                    title: t!("Notifications That Tidy Up"),
                    body: t!("Accept or decline in a notification, or in Linksy, and the notification goes away by itself."),
                },
                Highlight {
                    gtk: "starred-symbolic",
                    qt: "emblem-favorite",
                    title: t!("What's New, Right Here"),
                    body: t!("Each update now shows what changed, the first time it opens. It's also in the menu."),
                },
            ],
        },
        Release {
            version: "0.8.4",
            highlights: vec![
                Highlight {
                    gtk: "computer-symbolic",
                    qt: "computer",
                    title: t!("Linksy for KDE Plasma"),
                    body: t!("A Qt and Kirigami app in Breeze, with everything the GNOME one has. On Plasma, Linksy opens it."),
                },
                Highlight {
                    gtk: "network-server-symbolic",
                    qt: "network-server",
                    title: t!("Your Own Relay"),
                    body: t!("Away from home can go through a relay you host yourself: Preferences, Connection, Relay Server."),
                },
                Highlight {
                    gtk: "camera-web-symbolic",
                    qt: "camera-web",
                    title: t!("A Closer Look at the Webcam"),
                    body: t!("Preview what your apps see from the phone's camera, with its zoom, torch and focus right under it."),
                },
            ],
        },
    ]
}

/// The highlights of `version` and the versions after `since` up to it, newest first.
fn between(since: Option<&str>, version: &str) -> Vec<Highlight> {
    releases()
        .into_iter()
        .filter(|r| newer(version, r.version) || r.version == version)
        .filter(|r| since.is_none_or(|s| newer(r.version, s)))
        .flat_map(|r| r.highlights)
        .take(MOST)
        .collect()
}

/// Whether version `a` is newer than `b` ("0.10.0" after "0.9.3").
fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_default().join("linksy")
}

/// Where the last version whose news was seen is noted.
fn seen_file() -> PathBuf {
    config_dir().join("news-seen")
}

/// Whether Linksy was used here before this version: the service keeps its paired devices
/// and history in its data folder.
fn used_before() -> bool {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))).unwrap_or_default().join("linksy");
    ["trusted.json", "history.json"].iter().any(|f| std::fs::metadata(data.join(f)).is_ok_and(|m| m.len() > 2))
}

/// What to show now that `version` opens: its highlights (and those of versions skipped
/// since the last one seen) once, then nothing. Empty on a fresh install, and for
/// screenshots and development setups.
pub fn to_show(version: &str) -> Vec<Highlight> {
    if std::env::var_os("LINKSY_SCREENSHOT").is_some() || std::env::var_os("LINKSY_SOCKET").is_some() {
        return Vec::new();
    }
    let seen = std::fs::read_to_string(seen_file()).ok().map(|s| s.trim().to_string());
    match seen {
        Some(s) if !newer(version, &s) => Vec::new(),
        Some(s) => between(Some(&s), version),
        // Nothing noted: an update from before What's New (show this version's), or a
        // fresh install (nothing)
        None if used_before() => between(Some(previous(version)), version),
        None => {
            mark_seen(version);
            Vec::new()
        }
    }
}

/// The version before `version` that has highlights, so an update shows only its own.
fn previous(version: &str) -> &'static str {
    releases().iter().map(|r| r.version).find(|v| newer(version, v)).unwrap_or("0.0.0")
}

/// `version`'s highlights, for What's New in the menu.
pub fn latest(version: &str) -> Vec<Highlight> {
    between(Some(previous(version)), version)
}

/// Shown: don't show it again for this version.
pub fn mark_seen(version: &str) {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(seen_file(), version);
}

/// This version's full release notes.
pub fn notes_url(version: &str) -> String {
    format!("{REPO}/releases/tag/v{version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(!newer("0.8.4", "0.8.4"));
    }

    #[test]
    fn skipped_versions_add_up() {
        assert_eq!(between(Some("0.8.4"), "0.8.5").len(), 2);
        assert_eq!(between(Some("0.8.5"), "0.8.6").len(), 1);
        assert_eq!(between(Some("0.8.3"), "0.8.5").len(), 5);
        assert!(between(Some("0.8.5"), "0.8.5").is_empty());
    }
}
