//! Finish Setup's checks, shared by the GTK and the Qt app: the system pieces some features
//! need and a distribution may not have (the webcam in every app, video, screen sharing,
//! the clipboard, an open firewall, starting with the computer), and what's missing.
//! Everything that needs root, installing packages included, is done by
//! `linksy-system-setup` with pkexec, in one go: the computer asks for the password once,
//! on any desktop, with the distribution's own package tool (dnf, apt, zypper, pacman). The
//! same command is shown to run in a terminal instead.

use std::path::PathBuf;

use gio::prelude::*;
use linksy_i18n::t;

/// Which window asks: they show video with different GStreamer sinks.
#[derive(Clone, Copy, PartialEq)]
pub enum Ui {
    Gtk,
    Qt,
}

/// One thing to check.
#[derive(Clone)]
pub struct Item {
    pub id: &'static str,
    pub title: &'static str,
    pub why: &'static str,
    pub need: Need,
}

#[derive(Clone, PartialEq)]
pub enum Need {
    Ready,
    /// Distribution packages that bring it (then, for the camera, the device is made).
    Install(Vec<String>),
    /// Installed, but the camera device still has to be made (as root).
    SetUp,
    /// Nothing Linksy can do from here; what to do instead.
    Manual(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Family {
    Fedora,
    Debian,
    Arch,
    /// openSUSE (Tumbleweed, Leap) and SUSE Linux Enterprise.
    Suse,
    Other,
}

fn family() -> Family {
    // Inside a Flatpak, the host's release file is under /run/host.
    let text = std::fs::read_to_string("/run/host/os-release").or_else(|_| std::fs::read_to_string("/etc/os-release")).unwrap_or_default();
    family_of(&text, in_path)
}

/// From os-release, else from the package tool, as linksy-system-setup picks it: Artix
/// doesn't say it's Arch (desktop#54).
fn family_of(os_release: &str, has: impl Fn(&str) -> bool) -> Family {
    let ids: Vec<&str> = os_release
        .lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .flat_map(|v| v.trim_matches('"').split_whitespace())
        .collect();
    if ids.iter().any(|i| matches!(*i, "fedora" | "rhel" | "centos")) {
        Family::Fedora
    } else if ids.iter().any(|i| matches!(*i, "debian" | "ubuntu")) {
        Family::Debian
    } else if ids.iter().any(|i| matches!(*i, "arch" | "artix")) {
        Family::Arch
    } else if ids.iter().any(|i| i.starts_with("opensuse") || matches!(*i, "suse" | "sles")) {
        Family::Suse
    } else if has("dnf") {
        Family::Fedora
    } else if has("apt-get") {
        Family::Debian
    } else if has("zypper") {
        Family::Suse
    } else if has("pacman") {
        Family::Arch
    } else {
        Family::Other
    }
}

pub fn flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// Fedora Atomic (Silverblue, Kinoite): packages are layered with rpm-ostree, then a restart.
pub fn atomic() -> bool {
    std::path::Path::new("/run/ostree-booted").exists()
}

/// openSUSE's transactional systems (Aeon, Kalpa, MicroOS): a read-only root, so packages
/// go in with transactional-update, then a restart. zypper would fail there.
pub fn transactional() -> bool {
    let read_only_root = std::fs::read_to_string("/proc/mounts").is_ok_and(|m| {
        m.lines().any(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            f.get(1) == Some(&"/") && f.get(3).is_some_and(|o| o.split(',').any(|o| o == "ro"))
        })
    });
    read_only_root && std::path::Path::new("/usr/sbin/transactional-update").exists()
}

/// The running kernel's version (`uname -r`).
fn kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string()
}

/// The headers package for the running kernel on Arch and its derivatives: each kernel
/// package notes its own name (`pkgbase`: linux, linux-lts, linux-cachyos…), its headers
/// being that name with "-headers".
fn arch_headers(k: &str) -> String {
    let pkgbase = std::fs::read_to_string(format!("/usr/lib/modules/{k}/pkgbase")).ok().map(|s| s.trim().to_string()).filter(|s| s.starts_with("linux"));
    let pkgbase = pkgbase.unwrap_or_else(|| ["lts", "zen", "hardened"].iter().find(|f| k.ends_with(*f)).map_or("linux".to_string(), |f| format!("linux-{f}")));
    format!("{pkgbase}-headers")
}

/// The camera driver's packages; a driver built here (DKMS) needs the kernel's headers.
/// openSUSE ships it prebuilt and signed per kernel flavor (the end of `uname -r`: default,
/// longterm), so no building and no Secure Boot step there.
fn loopback_packages(fam: Family) -> Vec<String> {
    let k = kernel();
    match fam {
        Family::Fedora => vec!["akmod-v4l2loopback".into(), "v4l2loopback".into()],
        Family::Suse => {
            let flavor = k.rsplit('-').next().filter(|f| !f.is_empty()).unwrap_or("default");
            vec![format!("v4l2loopback-kmp-{flavor}"), "v4l2loopback-utils".into()]
        }
        Family::Debian => vec!["v4l2loopback-dkms".into(), "v4l2loopback-utils".into(), format!("linux-headers-{k}")],
        Family::Arch => vec!["v4l2loopback-dkms".into(), "v4l2loopback-utils".into(), arch_headers(&k)],
        Family::Other => Vec::new(),
    }
}

/// The package with `v4l2loopback-ctl`, for when the driver comes with the kernel
/// (CachyOS) but the tool to add a device doesn't.
fn loopback_tool(fam: Family) -> Option<&'static str> {
    match fam {
        Family::Fedora => Some("v4l2loopback"),
        Family::Debian | Family::Arch | Family::Suse => Some("v4l2loopback-utils"),
        Family::Other => None,
    }
}

fn has_element(names: &[&str]) -> bool {
    if gstreamer::init().is_err() {
        return false;
    }
    names.iter().any(|n| gstreamer::ElementFactory::find(n).is_some())
}

/// Whether apt knows a package (an installable version).
fn apt_has(package: &str) -> bool {
    std::process::Command::new("apt-cache")
        .args(["policy", package])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim().strip_prefix("Candidate:").is_some_and(|c| c.trim() != "(none)")))
}

fn in_path(cmd: &str) -> bool {
    // pkexec runs the helper with the system's PATH: sbin included, as here.
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).chain(["/usr/sbin".into(), "/sbin".into()]).any(|d| d.join(cmd).is_file()))
}

/// The "Linksy Camera" video device, if it exists.
pub fn camera_device() -> Option<PathBuf> {
    std::fs::read_dir("/sys/class/video4linux").ok()?.flatten().find_map(|e| {
        let name = std::fs::read_to_string(e.path().join("name")).ok()?;
        (name.trim() == "Linksy Camera").then(|| PathBuf::from("/dev").join(e.file_name()))
    })
}

/// What apps call a phone's camera: one "Linksy Camera" device for all phones when it
/// exists, else a PipeWire camera named after the phone.
pub fn camera_name(phone: &str) -> String {
    if camera_device().is_some() { "Linksy Camera".into() } else { format!("Linksy Camera ({phone})") }
}

/// Whether the desktop offers screen sharing to apps (the ScreenCast portal).
fn screencast_portal() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return true };
    bus.call_sync(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&("org.freedesktop.portal.ScreenCast", "version").to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

/// The portal backend that does screen sharing on this desktop.
fn portal_package() -> &'static str {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desktop.contains("kde") {
        "xdg-desktop-portal-kde"
    } else if desktop.contains("hyprland") {
        "xdg-desktop-portal-hyprland"
    } else if desktop.contains("niri") {
        // niri shares screens through GNOME's portal
        "xdg-desktop-portal-gnome"
    } else if ["sway", "river", "wlroots", "wayfire", "labwc"].iter().any(|d| desktop.contains(d)) {
        "xdg-desktop-portal-wlr"
    } else {
        "xdg-desktop-portal-gnome"
    }
}

fn output(argv: &[&str]) -> Option<String> {
    let out = std::process::Command::new(argv[0]).args(&argv[1..]).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether polkit lets this process do `action` right now without asking for a password.
/// Anything but a clear no (polkit missing, D-Bus down) counts as yes: the call then
/// decides for itself, as before.
fn polkit_allows(action: &str) -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else { return true };
    // The subject is this process: its pid, start time (field 22 of /proc/self/stat, after
    // the command's closing parenthesis) and uid, so polkit needn't look them up.
    let start_time = std::fs::read_to_string("/proc/self/stat").ok().and_then(|s| s.rsplit_once(") ")?.1.split_whitespace().nth(19)?.parse::<u64>().ok()).unwrap_or(0);
    let uid = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse::<i32>().ok()).unwrap_or(-1);
    let subject: std::collections::HashMap<&str, gio::glib::Variant> = [("pid", std::process::id().to_variant()), ("start-time", start_time.to_variant()), ("uid", uid.to_variant())].into();
    let details: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    // Flags 0: never ask the user from here.
    let args = (("unix-process", subject), action, details, 0u32, "").to_variant();
    let Ok(reply) = bus.call_sync(
        Some("org.freedesktop.PolicyKit1"),
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
        "CheckAuthorization",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    ) else {
        return true;
    };
    // (is_authorized, is_challenge, details)
    reply.child_value(0).child_value(0).get::<bool>().unwrap_or(true)
}

/// Whether a firewall would keep nearby devices out: None when there's none to open.
fn firewall_need() -> Option<Need> {
    if std::path::Path::new("/etc/linksy/firewall").exists() {
        return Some(Need::Ready);
    }
    // Whether firewalld runs, from systemd: `firewall-cmd --state` asks firewalld, and on
    // openSUSE even that needs the administrator's password (its state property falls
    // under firewalld's config action).
    let firewalld = std::process::Command::new("systemctl").args(["is-active", "--quiet", "firewalld.service"]).status().is_ok_and(|s| s.success());
    if in_path("firewall-cmd") && firewalld {
        // Reading the zone (firewalld's config.info action) needs no password on Fedora or
        // Debian; on openSUSE it needs the administrator's, at every start. Then nothing
        // is read: the firewall counts as closed until Linksy opens it itself (the marker
        // above), one password, when asked to. Anything unclear gets opened.
        if !polkit_allows("org.fedoraproject.FirewallD1.config.info") {
            return Some(Need::SetUp);
        }
        let Some(zone) = output(&["firewall-cmd", "--list-all"]) else { return Some(Need::SetUp) };
        let field = |name: &str| zone.lines().find_map(|l| l.trim().strip_prefix(&format!("{name}:")).map(|v| v.split_whitespace().map(str::to_string).collect::<Vec<_>>())).unwrap_or_default();
        let (services, ports) = (field("services"), field("ports"));
        let open = |port: u32| {
            ports.iter().any(|p| {
                let Some((range, "udp")) = p.split_once('/') else { return false };
                let (lo, hi) = range.split_once('-').unwrap_or((range, range));
                lo.parse::<u32>().is_ok_and(|lo| hi.parse::<u32>().is_ok_and(|hi| (lo..=hi).contains(&port)))
            })
        };
        let ready = services.iter().any(|s| s == "linksy") || (open(47200) && (open(5353) || services.iter().any(|s| s == "mdns")));
        return Some(if ready { Need::Ready } else { Need::SetUp });
    }
    let ufw = std::fs::read_to_string("/etc/ufw/ufw.conf").unwrap_or_default();
    if ufw.lines().any(|l| l.trim() == "ENABLED=yes") {
        return Some(Need::SetUp);
    }
    None
}

/// Whether the background service starts with every login: None when it isn't installed
/// as a user service (run from the source tree).
fn autostart_need() -> Option<Need> {
    let state = match std::process::Command::new("systemctl").args(["--user", "is-enabled", "linksyd.service"]).stderr(std::process::Stdio::null()).output() {
        Ok(s) if !s.stdout.is_empty() => s,
        // No systemd, or no user manager (Artix, Void: desktop#11): desktops start the service
        // from its XDG autostart entry, tiling compositors from a line in their config
        _ => return compositor_autostart().map(Need::Manual),
    };
    match String::from_utf8_lossy(&state.stdout).trim() {
        // Enabled, but tiling compositors seldom start systemd's graphical session, which
        // starts it: then one line in their config does
        "enabled" | "enabled-runtime" | "static" | "linked" | "alias" if !graphical_session() => compositor_autostart().map(Need::Manual).or(Some(Need::Ready)),
        "enabled" | "enabled-runtime" | "static" | "linked" | "alias" => Some(Need::Ready),
        "disabled" => Some(Need::SetUp),
        _ => None,
    }
}

/// Whether something shows notifications: a daemon running, or one D-Bus starts when asked.
fn notification_daemon() -> bool {
    const NAME: &str = "org.freedesktop.Notifications";
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return true };
    let call = |method: &str, args: Option<&gio::glib::Variant>| {
        bus.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", method, args, None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE)
    };
    let owned = call("NameHasOwner", Some(&(NAME,).to_variant())).ok().and_then(|v| v.get::<(bool,)>()).is_none_or(|(b,)| b);
    owned || call("ListActivatableNames", None).ok().and_then(|v| v.get::<(Vec<String>,)>()).is_some_and(|(names,)| names.iter().any(|n| n == NAME))
}

/// An AMD graphics card or chip (PCI vendor 0x1002).
fn amd_gpu() -> bool {
    std::fs::read_dir("/sys/class/drm").is_ok_and(|d| {
        d.flatten().any(|e| std::fs::read_to_string(e.path().join("device/vendor")).is_ok_and(|v| v.trim() == "0x1002"))
    })
}

fn graphical_session() -> bool {
    std::process::Command::new("systemctl").args(["--user", "is-active", "--quiet", "graphical-session.target"]).status().is_ok_and(|s| s.success())
}

/// The line that starts Linksy with a tiling compositor, for its config file.
fn compositor_autostart() -> Option<&'static str> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desktop.contains("hyprland") {
        Some(t!("Hyprland doesn't start Linksy at login by itself. Add this line to ~/.config/hypr/hyprland.conf: exec-once = linksyd"))
    } else if desktop.contains("sway") {
        Some(t!("Sway doesn't start Linksy at login by itself. Add this line to ~/.config/sway/config: exec linksyd"))
    } else if desktop.contains("niri") {
        Some(t!("niri doesn't start Linksy at login by itself. Add this line to ~/.config/niri/config.kdl: spawn-at-startup \"linksyd\""))
    } else if desktop.contains("river") {
        Some(t!("river doesn't start Linksy at login by itself. Add this line to ~/.config/river/init: linksyd &"))
    } else {
        None
    }
}

fn loopback_installed() -> bool {
    std::path::Path::new("/sys/module/v4l2loopback").exists()
        || std::process::Command::new("modinfo").arg("v4l2loopback").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn usb_rule() -> bool {
    ["/usr/lib/udev/rules.d/70-linksy-usb.rules", "/etc/udev/rules.d/70-linksy-usb.rules"].iter().any(|p| std::path::Path::new(p).exists())
}

/// What this computer has and lacks, right now, for the `ui` window.
pub fn check(ui: Ui) -> Vec<Item> {
    let fam = family();
    let pick = |fedora: &[&str], debian: &[&str], arch: &[&str], suse: &[&str], manual: &'static str| {
        let names = match fam {
            Family::Fedora => fedora,
            Family::Debian => debian,
            Family::Arch => arch,
            Family::Suse => suse,
            Family::Other => return Need::Manual(manual),
        };
        Need::Install(names.iter().map(|n| n.to_string()).collect())
    };
    let camera = if camera_device().is_some() {
        Need::Ready
    } else if flatpak() && fam == Family::Other {
        Need::Manual(t!("Install v4l2loopback from your distribution, then come back here."))
    } else if flatpak() {
        // The sandbox can't install a driver: one command outside it does (terminal_command)
        Need::Manual(t!("The Flatpak can't add this itself: copy the command under Prefer a Terminal? below, run it in a terminal, then come back here."))
    } else if loopback_installed() {
        // Some kernels bring the driver without its tool (CachyOS): add the tool first.
        match loopback_tool(fam) {
            Some(tool) if !in_path("v4l2loopback-ctl") => Need::Install(vec![tool.to_string()]),
            None if !in_path("v4l2loopback-ctl") => Need::Manual(t!("Install v4l2loopback from your distribution, then come back here.")),
            _ => Need::SetUp,
        }
    } else if fam == Family::Other {
        Need::Manual(t!("Install v4l2loopback from your distribution, then come back here."))
    } else {
        Need::Install(loopback_packages(fam))
    };
    let mut items = vec![Item {
        id: "camera",
        title: t!("Phone Camera in Every App"),
        why: t!("Browsers, Zoom and Discord only see this kind of camera. Without it, the phone's camera works in OBS and PipeWire apps only."),
        need: camera,
    }];
    // A Flatpak can't add a udev rule, and can't see whether the system has one.
    if !flatpak() {
        items.push(Item {
            id: "usb",
            title: t!("Phones by USB Cable"),
            why: t!("Plug a phone in for a faster webcam and microphone, with no Wi-Fi needed."),
            need: if usb_rule() { Need::Ready } else { Need::SetUp },
        });
    }
    // In a Flatpak, codecs and tools come with the runtime and the app.
    if !flatpak() {
        // H.264 both ways, and the window's video sink to show it: GTK's (on openSUSE, in
        // the gstreamer-plugins-rs bundle) or Qt Quick's.
        let (mut fedora, mut debian, mut arch, mut suse) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        if !(has_element(&["avdec_h264", "vah264dec", "openh264dec"]) && has_element(&["vah264lpenc", "vah264enc", "x264enc", "openh264enc"])) {
            fedora.extend(["gstreamer1-plugin-openh264", "gstreamer1-plugin-libav"]);
            debian.extend(["gstreamer1.0-libav", "gstreamer1.0-plugins-ugly"]);
            arch.extend(["gst-libav", "gst-plugins-ugly"]);
            suse.extend(["gstreamer-plugin-openh264", "gstreamer-plugins-libav"]);
        }
        match ui {
            // Ubuntu 24.04 has no package for it: the app draws frames itself there
            Ui::Gtk if !has_element(&["gtk4paintablesink"]) && !(family() == Family::Debian && !apt_has("gstreamer1.0-gtk4")) => {
                fedora.push("gstreamer1-plugin-gtk4");
                debian.push("gstreamer1.0-gtk4");
                arch.push("gst-plugin-gtk4");
                suse.push("gstreamer-plugins-rs");
            }
            Ui::Qt if !has_element(&["qml6glsink"]) => {
                fedora.push("gstreamer1-plugins-good-qt6");
                debian.push("gstreamer1.0-qt6");
                arch.push("gst-plugin-qml6");
                suse.push("gstreamer-plugins-good-qtqml6");
            }
            _ => {}
        }
        let manual = match ui {
            Ui::Gtk => t!("Install GStreamer's H.264 plugins (openh264 or libav) and its GTK 4 video sink (gtk4paintablesink) from your distribution."),
            Ui::Qt => t!("Install GStreamer's H.264 plugins (openh264 or libav) and its Qt 6 video sink (qml6glsink) from your distribution."),
        };
        items.push(Item {
            id: "video",
            title: t!("Video for Screens and Cameras"),
            why: t!("To send and show screens, second displays and the phone's camera."),
            need: if fedora.is_empty() { Need::Ready } else { pick(&fedora, &debian, &arch, &suse, manual) },
        });
        let (mut fedora, mut debian, mut arch, mut suse) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        if !has_element(&["pipewiresrc"]) {
            fedora.push("pipewire-gstreamer");
            debian.push("gstreamer1.0-pipewire");
            arch.push("gst-plugin-pipewire");
            suse.push("gstreamer-plugin-pipewire");
        }
        if !screencast_portal() {
            for list in [&mut fedora, &mut debian, &mut arch, &mut suse] {
                list.push(portal_package());
            }
        }
        items.push(Item {
            id: "pipewire",
            title: t!("Screen Sharing"),
            why: t!("Reading this computer's screen, and offering the phone's camera to apps."),
            need: if fedora.is_empty() { Need::Ready } else { pick(&fedora, &debian, &arch, &suse, t!("Install PipeWire's GStreamer plugin and your desktop's screen sharing portal.")) },
        });
        // Fedora's AMD video driver leaves H.264 encoding out (patents): RPM Fusion's full one
        // lets the GPU encode a second display at up to 60 fps, instead of the CPU at 30
        if fam == Family::Fedora && amd_gpu() && !has_element(&["vah264enc", "vah264lpenc"]) {
            items.push(Item {
                id: "gpu-video",
                title: t!("Smoother Second Display"),
                why: t!("Your graphics can encode video, but Fedora's driver leaves that out. RPM Fusion's full driver lets it send screens at up to 60 frames a second, with almost no processor."),
                need: Need::Install(vec!["mesa-va-drivers-freeworld".to_string()]),
            });
        }
        let clip = in_path("wl-copy") || in_path("xclip") || in_path("xsel");
        items.push(Item {
            id: "clipboard",
            title: t!("Clipboard From Your Phone"),
            why: t!("Putting text your phone shares on this computer's clipboard while Linksy's window is closed."),
            need: if clip { Need::Ready } else { pick(&["wl-clipboard"], &["wl-clipboard"], &["wl-clipboard"], &["wl-clipboard"], t!("Install wl-clipboard.")) },
        });
        // Tiling compositors start none; mako starts by itself once installed (D-Bus activation)
        if !notification_daemon() {
            items.push(Item {
                id: "notifications",
                title: t!("Notifications"),
                why: t!("Files, pairing codes and requests from your devices show as notifications, with their buttons."),
                need: pick(&["mako"], &["mako-notifier"], &["mako"], &["mako"], t!("Install a notification daemon (mako, dunst or swaync) and start it with your desktop.")),
            });
        }
        if let Some(need) = firewall_need() {
            items.push(Item {
                id: "firewall",
                title: t!("Found by Nearby Devices"),
                why: t!("The firewall lets your devices reach Linksy here (UDP 47200, and mDNS to find each other)."),
                need,
            });
        }
        if let Some(need) = autostart_need() {
            items.push(Item {
                id: "autostart",
                title: t!("Ready When You Log In"),
                why: t!("Linksy starts in the background with your computer, so phones can reach it with the window closed."),
                need,
            });
        }
    }
    items
}

/// What "Set Up Everything" can do from here (not the things to do by hand). On Fedora
/// Atomic and openSUSE's transactional systems, packages are for a terminal.
pub fn actionable(items: &[Item]) -> Vec<Item> {
    let layered = atomic() || transactional();
    items.iter().filter(|i| matches!(i.need, Need::SetUp) || (matches!(i.need, Need::Install(_)) && !layered)).cloned().collect()
}

/// How many things are missing (what the sidebar's Finish Setup shows).
pub fn missing(items: &[Item]) -> usize {
    items.iter().filter(|i| i.need != Need::Ready).count()
}

/// What a row tells about an item installed by hand on Fedora Atomic or openSUSE's
/// transactional systems; None elsewhere.
pub fn layered_hint(why: &str) -> Option<String> {
    if atomic() {
        Some(t!("{why} On Fedora Atomic: the command at the bottom, then restart.", why))
    } else if transactional() {
        Some(t!("{why} On openSUSE Aeon, Kalpa or MicroOS: the command at the bottom, then restart.", why))
    } else {
        None
    }
}

/// The root helper (`linksy-system-setup`): installed next to Linksy, or with the package
/// (in /usr/libexec, or /usr/lib on Arch), or in the source tree when run from it.
fn setup_script(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), PathBuf::from("/usr/libexec/linksy").join(name), PathBuf::from("/usr/lib/linksy").join(name), dir.join("../../data").join(name)].into_iter().find(|p| p.is_file())
}

/// What `linksy-system-setup` gets to set up `items` as root, in one run: RPM Fusion when
/// Fedora's camera driver needs it, the packages, then the camera, USB and firewall.
pub fn root_args(items: &[Item]) -> Vec<String> {
    let packages: Vec<String> = items.iter().flat_map(|i| if let Need::Install(p) = &i.need { p.clone() } else { Vec::new() }).collect();
    let mut args = Vec::new();
    let from_fusion = |p: &String| p.contains("v4l2loopback") || p.ends_with("-freeworld");
    if family() == Family::Fedora && packages.iter().any(from_fusion) && !std::path::Path::new("/etc/yum.repos.d/rpmfusion-free.repo").exists() {
        args.push("rpmfusion".to_string());
    }
    args.extend(packages.iter().map(|p| format!("pkg:{p}")));
    args.extend(items.iter().filter(|i| matches!(i.id, "camera" | "usb" | "firewall")).map(|i| i.id.to_string()));
    args
}

/// Whether setting up `items` turns on starting with the computer (the user's own
/// service: no password).
pub fn wants_autostart(items: &[Item]) -> bool {
    items.iter().any(|i| i.id == "autostart" && i.need != Need::Ready)
}

/// The same as Set Up Everything, to run in a terminal: for whoever prefers to, or when
/// the password dialog can't appear.
pub fn terminal_command(items: &[Item]) -> Option<String> {
    // In the Flatpak: the camera driver, by Linksy's own helper fetched for this version
    if flatpak() {
        let camera = items.iter().any(|i| i.id == "camera" && i.need != Need::Ready);
        let fam = family();
        if !camera || fam == Family::Other {
            return None;
        }
        let driver = Item { id: "camera", title: "", why: "", need: Need::Install(loopback_packages(fam)) };
        return Some(host_command(&root_args(&[driver])));
    }
    if atomic() || transactional() {
        let packages: Vec<String> = items.iter().flat_map(|i| if let Need::Install(p) = &i.need { p.clone() } else { Vec::new() }).collect();
        let tool = if atomic() { "rpm-ostree install" } else { "transactional-update --non-interactive pkg install" };
        return (!packages.is_empty()).then(|| format!("sudo {tool} {} && systemctl reboot", packages.join(" ")));
    }
    let args = root_args(&actionable(items));
    let script = setup_script("linksy-system-setup")?;
    let mut command = if args.is_empty() { String::new() } else { format!("sudo {} {}", script.display(), args.join(" ")) };
    if wants_autostart(items) {
        command = [command, "systemctl --user enable --now linksyd.service".into()].iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join(" && ");
    }
    (!command.is_empty()).then_some(command)
}

/// A command for a terminal outside the Flatpak: Linksy's setup helpers for this version,
/// from its public source, run with sudo for `tasks` (as linksy-system-setup takes them).
pub fn host_command(tasks: &[String]) -> String {
    let raw = format!("https://raw.githubusercontent.com/LinkSy-mg/linksy-desktop/v{}/data", env!("CARGO_PKG_VERSION"));
    let helpers = ["linksy-system-setup", "linksy-camera-setup", "linksy-usb-setup"];
    let fetch: Vec<String> = helpers.iter().map(|h| format!("-fsSLO {raw}/{h}")).collect();
    format!("cd /tmp && curl {} && chmod +x {} && sudo ./linksy-system-setup {}", fetch.join(" "), helpers.join(" "), tasks.join(" "))
}

/// Why setting up stopped.
pub enum Failure {
    Message(String),
    /// The camera driver waits for Secure Boot's approval at the next restart; with the
    /// code to type in the blue screen then, when there is one.
    Restart(Option<String>),
}

/// The helper's step (it speaks English) in the language in use.
pub fn step_text(step: &str) -> String {
    match step.trim() {
        "Turning on RPM Fusion" => t!("Turning on RPM Fusion").into(),
        "Building the camera driver (a few minutes)" => t!("Building the camera driver (a few minutes)").into(),
        "Letting nearby devices find this computer" => t!("Letting nearby devices find this computer").into(),
        "Making the Linksy Camera" => t!("Making the Linksy Camera").into(),
        "Building the camera driver again, signed with the approved key" => t!("Building the camera driver again, signed with the approved key").into(),
        "Letting Linksy use phones plugged in by cable" => t!("Letting Linksy use phones plugged in by cable").into(),
        s => match s.strip_prefix("Installing ") {
            Some(packages) => t!("Installing {packages}", packages),
            None => s.to_string(),
        },
    }
}

/// Reads the helper's output as it comes: steps to show, the Secure Boot code, and the last
/// lines for when it fails.
#[derive(Default)]
pub struct Progress {
    enroll: Option<String>,
    last: std::collections::VecDeque<String>,
    /// pkexec found no polkit agent to ask for the password.
    pub no_agent: bool,
}

impl Progress {
    /// One line of output; the step to show now, if it is one.
    pub fn line(&mut self, line: &str) -> Option<String> {
        if let Some(step) = line.strip_prefix("STEP ") {
            return Some(t!("{step}…", step = step_text(step)));
        }
        if line.contains("No authentication agent found") {
            self.no_agent = true;
        }
        if let Some(code) = line.strip_prefix("ENROLL ") {
            self.enroll = Some(code.trim().to_string());
        } else if !line.trim().is_empty() {
            self.last.push_back(line.trim().to_string());
            if self.last.len() > 3 {
                self.last.pop_front();
            }
        }
        None
    }

    /// What the helper's exit status means.
    pub fn finish(self, status: i32) -> Result<(), Failure> {
        match status {
            0 => Ok(()),
            4 => Err(Failure::Restart(self.enroll)),
            126 | 127 => Err(Failure::Message(t!("Not set up: the password wasn't given.").into())),
            _ => Err(Failure::Message(t!("Setup didn't finish: {details}", details = self.last.into_iter().collect::<Vec<_>>().join(" ")))),
        }
    }
}

/// pkexec and the helper, with `args`: the command line to run (as root, one password).
pub fn root_command(args: &[String]) -> Result<Vec<std::ffi::OsString>, Failure> {
    let script = setup_script("linksy-system-setup").ok_or_else(|| Failure::Message(t!("Linksy's setup helper is missing; reinstall Linksy.").into()))?;
    let mut argv = vec!["pkexec".into(), script.into_os_string()];
    argv.extend(args.iter().map(Into::into));
    Ok(argv)
}

/// Run `linksy-system-setup` as root and wait, telling `step` what it does as it goes. For
/// a thread of its own: it blocks until the helper ends.
pub fn run_as_root(args: &[String], mut step: impl FnMut(String)) -> Result<(), Failure> {
    let (status, progress) = attempt_as_root(args, &mut step)?;
    // Hyprland, Sway and others start no password prompt (polkit agent): start an installed
    // one, then ask again
    if progress.no_agent {
        if !start_polkit_agent() {
            return Err(no_agent());
        }
        let (status, progress) = attempt_as_root(args, &mut step)?;
        return progress.finish(status);
    }
    progress.finish(status)
}

fn attempt_as_root(args: &[String], step: &mut impl FnMut(String)) -> Result<(i32, Progress), Failure> {
    use std::io::BufRead;
    let argv = root_command(args)?;
    // stderr into the same pipe as stdout, as the GTK app reads it
    let (read, write) = std::io::pipe().map_err(|e| Failure::Message(e.to_string()))?;
    let write_err = write.try_clone().map_err(|e| Failure::Message(e.to_string()))?;
    let mut child = std::process::Command::new(&argv[0]).args(&argv[1..]).stdout(write).stderr(write_err).spawn().map_err(|e| Failure::Message(e.to_string()))?;
    let mut progress = Progress::default();
    step(t!("Waiting for your password…").into());
    for line in std::io::BufReader::new(read).lines().map_while(Result::ok) {
        if let Some(s) = progress.line(&line) {
            step(s);
        }
    }
    let status = child.wait().map_err(|e| Failure::Message(e.to_string()))?;
    Ok((status.code().unwrap_or(1), progress))
}

/// No polkit agent runs and none is installed.
pub fn no_agent() -> Failure {
    Failure::Message(t!("This desktop has no password prompt running (a polkit agent, like hyprpolkitagent or polkit-gnome). Run the command under Prefer a Terminal? instead, or install and start one.").into())
}

/// Start the first polkit agent installed here, for this session. Whether one started.
/// Blocks for a moment while it registers.
pub fn start_polkit_agent() -> bool {
    const AGENTS: [&str; 14] = [
        "/usr/libexec/hyprpolkitagent",
        "/usr/lib/hyprpolkitagent/hyprpolkitagent",
        "/usr/libexec/polkit-gnome-authentication-agent-1",
        "/usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1",
        "/usr/lib/policykit-1-gnome/polkit-gnome-authentication-agent-1",
        "/usr/libexec/polkit-kde-authentication-agent-1",
        "/usr/lib/x86_64-linux-gnu/libexec/polkit-kde-authentication-agent-1",
        "/usr/lib/polkit-kde-authentication-agent-1",
        "/usr/libexec/polkit-mate-authentication-agent-1",
        "/usr/lib/mate-polkit/polkit-mate-authentication-agent-1",
        "/usr/libexec/xfce-polkit",
        "/usr/lib/xfce-polkit/xfce-polkit",
        "/usr/bin/lxpolkit",
        "/usr/bin/soteria",
    ];
    let Some(agent) = AGENTS.iter().find(|p| std::path::Path::new(p).is_file()) else { return false };
    let started = std::process::Command::new(agent)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok();
    // It registers with polkit in a moment
    std::thread::sleep(std::time::Duration::from_millis(1500));
    started
}

/// Turn on the background service at every login (no password: it's the user's own).
pub fn enable_autostart() -> Result<(), Failure> {
    let ok = std::process::Command::new("systemctl").args(["--user", "enable", "--now", "linksyd.service"]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(Failure::Message(t!("Couldn't turn on starting with your computer.").into())) }
}

/// Whether phones can be used by cable here: Err says why not.
pub fn usb_ready() -> Result<(), String> {
    if flatpak() {
        // The Flatpak can't add the USB rule itself: one command outside it does
        return Err(t!("Allow it once by running this in a terminal (outside the Flatpak), then come back: {command}", command = host_command(&["usb".to_string()])));
    }
    if usb_rule() { Ok(()) } else { Err(t!("Allow it once: your computer asks for your password.").into()) }
}

/// What a Secure Boot restart asks for, in words: the exact steps, and the code to type.
pub fn restart_steps(code: Option<&str>) -> String {
    match code {
        Some(code) => t!("Secure Boot asks you to approve the camera driver once.\n\n1. Restart your computer.\n2. A blue screen appears: choose Enroll MOK, then Continue, then Yes.\n3. Type this code: {code}\n4. Choose Reboot.\n\nLinksy Camera is then ready, nothing else to do. Write the code down: it's needed on that blue screen.", code),
        None => t!("Restart your computer to finish. If a blue screen appears, choose Enroll MOK, then Continue, Yes, and Reboot. Then open Finish Setup again.").to_string(),
    }
}

/// Where Linksy notes which version last offered Finish Setup by itself.
fn offered_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_default();
    config.join("linksy").join("setup-offered")
}

/// The first time each `version` starts with something to set up: true once, to offer it.
pub fn offer_now(ui: Ui, version: &str) -> bool {
    if std::env::var_os("LINKSY_SCREENSHOT").is_some() || std::env::var_os("LINKSY_SOCKET").is_some() {
        return false;
    }
    if std::fs::read_to_string(offered_file()).is_ok_and(|v| v.trim() == version) || actionable(&check(ui)).is_empty() {
        return false;
    }
    if let Some(dir) = offered_file().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(offered_file(), version);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributions_that_hide_their_base_go_by_their_package_tool() {
        let artix = "NAME=\"Artix Linux\"\nPRETTY_NAME=\"Artix Linux\"\nID=\"artix\"\nBUILD_ID=\"rolling\"\n";
        assert_eq!(family_of(artix, |_| false), Family::Arch);
        assert_eq!(family_of("ID=newdistro\n", |c| c == "pacman"), Family::Arch);
        assert_eq!(family_of("ID=newdistro\n", |c| c == "zypper"), Family::Suse);
        assert_eq!(family_of("ID=newdistro\n", |_| false), Family::Other);
        assert_eq!(family_of("ID=ubuntu\nID_LIKE=debian\n", |c| c == "pacman"), Family::Debian);
        assert_eq!(family_of("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n", |_| false), Family::Suse);
    }

    #[test]
    fn arch_headers_without_pkgbase_follow_the_name() {
        assert_eq!(arch_headers("0.0.0-none-lts"), "linux-lts-headers");
        assert_eq!(arch_headers("0.0.0-none"), "linux-headers");
    }

    #[test]
    fn progress_reads_steps_and_codes() {
        let mut p = Progress::default();
        assert!(p.line("STEP Making the Linksy Camera").is_some());
        assert!(p.line("ENROLL 12345678").is_none());
        assert!(matches!(p.finish(4), Err(Failure::Restart(Some(c))) if c == "12345678"));
    }
}
