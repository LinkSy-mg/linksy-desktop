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

/// One thing worth noticing.
#[derive(Clone)]
pub struct Highlight {
    /// A symbolic icon: `gtk` from Adwaita's set, `qt` from Breeze's.
    pub gtk: &'static str,
    pub qt: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

/// One version's highlights.
#[derive(Clone)]
pub struct Release {
    pub version: &'static str,
    pub highlights: Vec<Highlight>,
}

/// Newest first.
fn releases() -> Vec<Release> {
    vec![
        Release {
            version: "0.9.7",
            highlights: vec![Highlight {
                gtk: "mg.diary.linksy.Linksy-tray-symbolic",
                qt: "mg.diary.linksy.Linksy-tray-symbolic",
                title: t!("Linksy in Your Tray"),
                body: t!("Your devices and what they can do, a click away in the top bar or panel. On GNOME it needs the AppIndicator extension."),
            }],
        },
        Release {
            version: "0.9.5",
            highlights: vec![Highlight {
                gtk: "video-display-symbolic",
                qt: "video-display",
                title: t!("Smoother Second Display"),
                body: t!("Screens go through the graphics chip when it can encode video, at up to 60 frames a second. On Fedora with AMD graphics, Finish Setup adds the driver it needs."),
            }],
        },
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
        Release {
            version: "0.8.3",
            highlights: vec![
                Highlight {
                    gtk: "emblem-shared-symbolic",
                    qt: "emblem-shared",
                    title: t!("Linksy Is Open Source"),
                    body: t!("Its code is public under the GNU AGPL: read it, build it, and help make it better."),
                },
                Highlight {
                    gtk: "system-software-install-symbolic",
                    qt: "system-software-install",
                    title: t!("Packages for openSUSE"),
                    body: t!("Each release now has an .rpm for openSUSE Tumbleweed, beside the Fedora one."),
                },
            ],
        },
        Release {
            version: "0.8.0",
            highlights: vec![
                Highlight {
                    gtk: "preferences-desktop-locale-symbolic",
                    qt: "preferences-desktop-locale",
                    title: t!("Linksy Speaks Your Language"),
                    body: t!("Ten languages, from English and Français to 日本語 and Malagasy, following your computer's."),
                },
                Highlight {
                    gtk: "media-removable-symbolic",
                    qt: "drive-removable-media-usb",
                    title: t!("Connect by USB Cable"),
                    body: t!("A guide on a phone's page uses the cable for a faster webcam and microphone, with no Wi-Fi needed."),
                },
                Highlight {
                    gtk: "audio-input-microphone-symbolic",
                    qt: "audio-input-microphone",
                    title: t!("See the Microphone Level"),
                    body: t!("While a phone's microphone is on, its card shows how loud it is, and Listen plays it here."),
                },
            ],
        },
    ]
}

/// The versions after `since` up to `version` (included) that have highlights, newest first.
fn between(since: Option<&str>, version: &str) -> Vec<Release> {
    releases()
        .into_iter()
        .filter(|r| newer(version, r.version) || r.version == version)
        .filter(|r| since.is_none_or(|s| newer(r.version, s)))
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

/// What to show now that `version` opens: its highlights and those of every version since
/// the last one seen, grouped by version, once; then nothing. Empty on a fresh install,
/// and for screenshots and development setups.
pub fn to_show(version: &str) -> Vec<Release> {
    if std::env::var_os("LINKSY_SCREENSHOT").is_some() || std::env::var_os("LINKSY_SOCKET").is_some() {
        return Vec::new();
    }
    let seen = std::fs::read_to_string(seen_file()).ok().map(|s| s.trim().to_string());
    match seen {
        Some(s) if !newer(version, &s) => Vec::new(),
        Some(s) => between(Some(&s), version),
        // Nothing noted: an update from before What's New (0.8.4), so from an unknown
        // version: everything there is; or a fresh install (nothing)
        None if used_before() => between(None, version),
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
pub fn latest(version: &str) -> Vec<Release> {
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
        let count = |r: Vec<Release>| (r.len(), r.iter().map(|r| r.highlights.len()).sum::<usize>());
        assert_eq!(count(between(Some("0.8.4"), "0.8.5")), (1, 2));
        assert_eq!(count(between(Some("0.8.5"), "0.8.6")), (1, 1));
        assert_eq!(count(between(Some("0.8.3"), "0.8.5")), (2, 5));
        assert!(between(Some("0.8.5"), "0.8.5").is_empty());
        // From 0.8.0 to 0.9.0: every version after it, newest first, nothing capped
        let from_080 = between(Some("0.8.0"), "0.9.0");
        assert_eq!(from_080.first().map(|r| r.version), Some("0.9.0"));
        assert_eq!(from_080.last().map(|r| r.version), Some("0.8.3"));
        assert_eq!(between(None, "0.9.0").last().map(|r| r.version), Some("0.8.0"));
    }
}
