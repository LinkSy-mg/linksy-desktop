//! Finish Setup: the system pieces some features need and a distribution may not have
//! (the webcam in every app, video, screen sharing, the clipboard, an open firewall,
//! starting with the computer), what's missing, and one click to add it all ("Set Up
//! Everything"), or each on its own. Everything that needs root, installing packages
//! included, is done by `linksy-system-setup` with pkexec, in one go: the computer asks
//! for the password once, on any desktop (GNOME, KDE or other), with the distribution's
//! own package tool (dnf, apt, zypper, pacman). The same command is shown to run in a terminal
//! instead. It opens by itself the first time each version of Linksy starts with
//! something missing.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use linksy_i18n::t;

use super::Window;

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

#[derive(Clone, Copy, PartialEq)]
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
    let ids: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .flat_map(|v| v.trim_matches('"').split_whitespace())
        .collect();
    if ids.iter().any(|i| matches!(*i, "fedora" | "rhel" | "centos")) {
        Family::Fedora
    } else if ids.iter().any(|i| matches!(*i, "debian" | "ubuntu")) {
        Family::Debian
    } else if ids.contains(&"arch") {
        Family::Arch
    } else if ids.iter().any(|i| i.starts_with("opensuse") || matches!(*i, "suse" | "sles")) {
        Family::Suse
    } else {
        Family::Other
    }
}

fn flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// Fedora Atomic (Silverblue, Kinoite): packages are layered with rpm-ostree, then a restart.
fn atomic() -> bool {
    std::path::Path::new("/run/ostree-booted").exists()
}

/// The running kernel's version (`uname -r`).
fn kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string()
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
        Family::Arch => {
            let headers = ["lts", "zen", "hardened"].iter().find(|f| k.ends_with(*f)).map_or("linux-headers".to_string(), |f| format!("linux-{f}-headers"));
            vec!["v4l2loopback-dkms".into(), "v4l2loopback-utils".into(), headers]
        }
        Family::Other => Vec::new(),
    }
}

fn has_element(names: &[&str]) -> bool {
    names.iter().any(|n| gstreamer::ElementFactory::find(n).is_some())
}

fn in_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
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
    } else if ["sway", "river", "wlroots", "wayfire", "labwc", "niri"].iter().any(|d| desktop.contains(d)) {
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
    let subject: std::collections::HashMap<&str, glib::Variant> = [("pid", std::process::id().to_variant()), ("start-time", start_time.to_variant()), ("uid", uid.to_variant())].into();
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
    let state = std::process::Command::new("systemctl").args(["--user", "is-enabled", "linksyd.service"]).stderr(std::process::Stdio::null()).output().ok()?;
    match String::from_utf8_lossy(&state.stdout).trim() {
        "enabled" | "enabled-runtime" | "static" | "linked" | "alias" => Some(Need::Ready),
        "disabled" => Some(Need::SetUp),
        _ => None,
    }
}

fn loopback_installed() -> bool {
    std::path::Path::new("/sys/module/v4l2loopback").exists()
        || std::process::Command::new("modinfo").arg("v4l2loopback").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// What this computer has and lacks, right now.
pub fn check() -> Vec<Item> {
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
    } else if flatpak() {
        Need::Manual(t!("A Flatpak can't add this. Install Linksy's .deb or .rpm instead, or v4l2loopback and run linksy-camera-setup once."))
    } else if loopback_installed() {
        Need::SetUp
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
        let rule = ["/usr/lib/udev/rules.d/70-linksy-usb.rules", "/etc/udev/rules.d/70-linksy-usb.rules"].iter().any(|p| std::path::Path::new(p).exists());
        items.push(Item {
            id: "usb",
            title: t!("Phones by USB Cable"),
            why: t!("Plug a phone in for a faster webcam and microphone, with no Wi-Fi needed."),
            need: if rule { Need::Ready } else { Need::SetUp },
        });
    }
    // In a Flatpak, codecs and tools come with the runtime and the app.
    if !flatpak() {
        // H.264 both ways, and GTK's video sink to show it in a window (on openSUSE, in
        // the gstreamer-plugins-rs bundle).
        let (mut fedora, mut debian, mut arch, mut suse) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        if !(has_element(&["avdec_h264", "vah264dec", "openh264dec"]) && has_element(&["vah264lpenc", "vah264enc", "x264enc", "openh264enc"])) {
            fedora.extend(["gstreamer1-plugin-openh264", "gstreamer1-plugin-libav"]);
            debian.extend(["gstreamer1.0-libav", "gstreamer1.0-plugins-ugly"]);
            arch.extend(["gst-libav", "gst-plugins-ugly"]);
            suse.extend(["gstreamer-plugin-openh264", "gstreamer-plugins-libav"]);
        }
        if !has_element(&["gtk4paintablesink"]) {
            fedora.push("gstreamer1-plugin-gtk4");
            debian.push("gstreamer1.0-gtk4");
            arch.push("gst-plugin-gtk4");
            suse.push("gstreamer-plugins-rs");
        }
        items.push(Item {
            id: "video",
            title: t!("Video for Screens and Cameras"),
            why: t!("To send and show screens, second displays and the phone's camera."),
            need: if fedora.is_empty() { Need::Ready } else { pick(&fedora, &debian, &arch, &suse, t!("Install GStreamer's H.264 plugins (openh264 or libav) and its GTK 4 video sink (gtk4paintablesink) from your distribution.")) },
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
        let clip = in_path("wl-copy") || in_path("xclip") || in_path("xsel");
        items.push(Item {
            id: "clipboard",
            title: t!("Clipboard From Your Phone"),
            why: t!("Putting text your phone shares on this computer's clipboard while Linksy's window is closed."),
            need: if clip { Need::Ready } else { pick(&["wl-clipboard"], &["wl-clipboard"], &["wl-clipboard"], &["wl-clipboard"], t!("Install wl-clipboard.")) },
        });
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
/// Atomic, packages are for rpm-ostree, in a terminal.
fn actionable(items: &[Item]) -> Vec<Item> {
    items.iter().filter(|i| matches!(i.need, Need::SetUp) || (matches!(i.need, Need::Install(_)) && !atomic())).cloned().collect()
}

/// How many things are missing (what the sidebar's Finish Setup shows).
pub fn missing(items: &[Item]) -> usize {
    items.iter().filter(|i| i.need != Need::Ready).count()
}

/// The root helper (`linksy-system-setup`): installed next to Linksy, or with the package,
/// or in the source tree when run from it.
fn setup_script(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), PathBuf::from("/usr/libexec/linksy").join(name), dir.join("../../data").join(name)].into_iter().find(|p| p.is_file())
}

/// What `linksy-system-setup` gets to set up `items` as root, in one run: RPM Fusion when
/// Fedora's camera driver needs it, the packages, then the camera, USB and firewall.
fn root_args(items: &[Item]) -> Vec<String> {
    let packages: Vec<String> = items.iter().flat_map(|i| if let Need::Install(p) = &i.need { p.clone() } else { Vec::new() }).collect();
    let mut args = Vec::new();
    if family() == Family::Fedora && packages.iter().any(|p| p.contains("v4l2loopback")) && !std::path::Path::new("/etc/yum.repos.d/rpmfusion-free.repo").exists() {
        args.push("rpmfusion".to_string());
    }
    args.extend(packages.iter().map(|p| format!("pkg:{p}")));
    args.extend(items.iter().filter(|i| matches!(i.id, "camera" | "usb" | "firewall")).map(|i| i.id.to_string()));
    args
}

/// The same as Set Up Everything, to run in a terminal: for whoever prefers to, or when
/// the password dialog can't appear.
fn terminal_command(items: &[Item]) -> Option<String> {
    if atomic() {
        let packages: Vec<String> = items.iter().flat_map(|i| if let Need::Install(p) = &i.need { p.clone() } else { Vec::new() }).collect();
        return (!packages.is_empty()).then(|| format!("sudo rpm-ostree install {} && systemctl reboot", packages.join(" ")));
    }
    let args = root_args(&actionable(items));
    let script = setup_script("linksy-system-setup")?;
    let mut command = if args.is_empty() { String::new() } else { format!("sudo {} {}", script.display(), args.join(" ")) };
    if items.iter().any(|i| i.id == "autostart" && i.need != Need::Ready) {
        command = [command, "systemctl --user enable --now linksyd.service".into()].iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join(" && ");
    }
    (!command.is_empty()).then_some(command)
}

/// Why setting up stopped.
enum Failure {
    Message(String),
    /// The camera driver waits for Secure Boot's approval at the next restart; with the
    /// code to type in the blue screen then, when there is one.
    Restart(Option<String>),
}

/// The helper's step (it speaks English) in the language in use.
fn step_text(step: &str) -> String {
    match step.trim() {
        "Turning on RPM Fusion" => t!("Turning on RPM Fusion").into(),
        "Building the camera driver (a few minutes)" => t!("Building the camera driver (a few minutes)").into(),
        "Letting nearby devices find this computer" => t!("Letting nearby devices find this computer").into(),
        "Making the Linksy Camera" => t!("Making the Linksy Camera").into(),
        "Letting Linksy use phones plugged in by cable" => t!("Letting Linksy use phones plugged in by cable").into(),
        s => match s.strip_prefix("Installing ") {
            Some(packages) => t!("Installing {packages}", packages),
            None => s.to_string(),
        },
    }
}

/// Run `linksy-system-setup` as root (one password for all of it), showing each step.
async fn run_as_root(args: &[String], progress: &gtk::Label) -> Result<(), Failure> {
    let script = setup_script("linksy-system-setup").ok_or_else(|| Failure::Message(t!("Linksy's setup helper is missing; reinstall Linksy.").into()))?;
    let mut argv = vec![std::ffi::OsStr::new("pkexec"), script.as_os_str()];
    argv.extend(args.iter().map(std::ffi::OsStr::new));
    let run = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE).map_err(|e| Failure::Message(e.to_string()))?;
    let lines = gio::DataInputStream::new(&run.stdout_pipe().expect("stdout pipe"));
    let (mut enroll, mut last) = (None, std::collections::VecDeque::new());
    progress.set_label(t!("Waiting for your password…"));
    progress.set_visible(true);
    while let Ok(Some(line)) = lines.read_line_utf8_future(glib::Priority::DEFAULT).await {
        if let Some(step) = line.strip_prefix("STEP ") {
            progress.set_label(&t!("{step}…", step = step_text(step)));
        } else if let Some(code) = line.strip_prefix("ENROLL ") {
            enroll = Some(code.trim().to_string());
        } else if !line.trim().is_empty() {
            last.push_back(line.trim().to_string());
            if last.len() > 3 {
                last.pop_front();
            }
        }
    }
    let _ = run.wait_future().await;
    progress.set_visible(false);
    match run.exit_status() {
        0 => Ok(()),
        4 => Err(Failure::Restart(enroll)),
        126 | 127 => Err(Failure::Message(t!("Not set up: the password wasn't given.").into())),
        _ => Err(Failure::Message(t!("Setup didn't finish: {details}", details = last.into_iter().collect::<Vec<_>>().join(" ")))),
    }
}

/// Whether phones can be used by cable here: Err says why not.
pub(super) fn usb_ready() -> Result<(), String> {
    if flatpak() {
        return Err(t!("The Flatpak can't use USB cables. Install Linksy's .rpm or .deb for this.").into());
    }
    let rule = ["/usr/lib/udev/rules.d/70-linksy-usb.rules", "/etc/udev/rules.d/70-linksy-usb.rules"].iter().any(|p| std::path::Path::new(p).exists());
    if rule { Ok(()) } else { Err(t!("Allow it once: your computer asks for your password.").into()) }
}

/// Let Linksy use phones by cable (the udev rule), as root.
pub(super) async fn allow_usb() -> Result<(), String> {
    match run_as_root(&["usb".to_string()], &gtk::Label::new(None)).await {
        Ok(()) => Ok(()),
        Err(Failure::Message(m)) => Err(m),
        Err(Failure::Restart(_)) => Err(t!("Restart your computer, then try again.").into()),
    }
}

/// Turn on the background service at every login (no password: it's the user's own).
async fn enable_autostart() -> Result<(), Failure> {
    let failed = || Failure::Message(t!("Couldn't turn on starting with your computer.").into());
    let run = gio::Subprocess::newv(&["systemctl".as_ref(), "--user".as_ref(), "enable".as_ref(), "--now".as_ref(), "linksyd.service".as_ref()], gio::SubprocessFlags::STDERR_SILENCE).map_err(|_| failed())?;
    run.wait_future().await.map_err(|_| failed())?;
    if run.exit_status() == 0 { Ok(()) } else { Err(failed()) }
}

/// Where Linksy notes which version last offered Finish Setup by itself.
fn offered_file() -> PathBuf {
    glib::user_config_dir().join("linksy").join("setup-offered")
}

/// The parts of the Finish Setup dialog that change as things get set up.
#[derive(Clone)]
struct SetupView {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    toasts: adw::ToastOverlay,
    everything: gtk::Button,
    progress: gtk::Label,
    /// "Prefer a Terminal?", in its own list.
    terminal: gtk::ListBox,
    command: gtk::Label,
}

impl Window {
    pub(super) fn refresh_setup(&self) {
        let n = missing(&check());
        // Screenshots (LINKSY_SCREENSHOT) show the app as on a finished setup.
        self.setup_button.set_visible(n > 0 && std::env::var_os("LINKSY_SCREENSHOT").is_none());
        self.setup_button.set_label(&if n == 1 { t!("Finish Setup · 1 thing").to_string() } else { t!("Finish Setup · {n} things", n) });
    }

    /// The first time each version starts with something to set up: offer it.
    pub(super) fn offer_setup(self: &Rc<Self>) {
        if std::env::var_os("LINKSY_SCREENSHOT").is_some() || std::env::var_os("LINKSY_SOCKET").is_some() {
            return;
        }
        let version = env!("CARGO_PKG_VERSION");
        if std::fs::read_to_string(offered_file()).is_ok_and(|v| v.trim() == version) || actionable(&check()).is_empty() {
            return;
        }
        let _ = std::fs::create_dir_all(offered_file().parent().unwrap_or(&glib::user_config_dir()));
        let _ = std::fs::write(offered_file(), version);
        self.show_setup();
    }

    pub fn show_setup(self: &Rc<Self>) {
        let dialog = adw::Dialog::builder().title(t!("Finish Setup")).content_width(560).build();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let intro = gtk::Label::builder()
            .label(t!("A few parts of your system make every feature work. Linksy adds what's missing in one go; your computer asks for your password once."))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(6).margin_bottom(24).margin_start(24).margin_end(24).build();
        let everything = gtk::Button::builder().label(t!("Set Up Everything")).halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
        let progress = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).visible(false).build();
        // The same, by hand: one command to paste in a terminal.
        let command = gtk::Label::builder().wrap(true).wrap_mode(gtk::pango::WrapMode::Char).selectable(true).xalign(0.0).css_classes(["monospace"]).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
        let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text(t!("Copy")).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let terminal = adw::ExpanderRow::builder().title(t!("Prefer a Terminal?")).subtitle(t!("Run this instead, then reopen Finish Setup")).build();
        terminal.add_suffix(&copy);
        terminal.add_row(&command);
        let terminal_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        terminal_list.append(&terminal);
        body.append(&intro);
        body.append(&everything);
        body.append(&progress);
        body.append(&list);
        body.append(&terminal_list);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).child(&body).build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&scroller));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        dialog.set_child(Some(&toasts));
        let v = SetupView { dialog: dialog.clone(), list, toasts, everything: everything.clone(), progress, terminal: terminal_list.clone(), command: command.clone() };
        let c = command.clone();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&c.label());
            b.set_icon_name("object-select-symbolic");
        });
        self.fill_setup(&v);
        let (w, view) = (self.clone(), v.clone());
        everything.connect_clicked(move |_| w.clone().fix(view.clone(), actionable(&check())));
        let w = self.clone();
        dialog.connect_closed(move |_| w.refresh_setup());
        dialog.present(Some(&self.window));
    }

    fn fill_setup(self: &Rc<Self>, v: &SetupView) {
        while let Some(row) = v.list.first_child() {
            v.list.remove(&row);
        }
        let items = check();
        v.everything.set_visible(actionable(&items).len() > 1);
        v.everything.set_sensitive(true);
        v.everything.set_label(t!("Set Up Everything"));
        let command = terminal_command(&items);
        v.terminal.set_visible(command.is_some());
        v.command.set_label(command.as_deref().unwrap_or_default());
        for item in items {
            let row = adw::ActionRow::builder().title(item.title).subtitle(item.why).subtitle_lines(4).build();
            match &item.need {
                Need::Ready => {
                    let ok = gtk::Image::builder().icon_name("object-select-symbolic").css_classes(["success"]).tooltip_text(t!("Ready")).build();
                    row.add_suffix(&ok);
                }
                Need::Manual(what) => {
                    row.set_subtitle(&glib::markup_escape_text(&format!("{} {what}", item.why)));
                    row.add_prefix(&gtk::Image::builder().icon_name("dialog-information-symbolic").css_classes(["dim-label"]).build());
                }
                Need::Install(_) if atomic() => {
                    row.set_subtitle(&glib::markup_escape_text(&t!("{why} On Fedora Atomic: the command at the bottom, then restart.", why = item.why)));
                    row.add_prefix(&gtk::Image::builder().icon_name("dialog-information-symbolic").css_classes(["dim-label"]).build());
                }
                need => {
                    let label = if matches!(need, Need::SetUp) { t!("Set Up") } else { t!("Install") };
                    let button = gtk::Button::builder().label(label).valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
                    row.add_suffix(&button);
                    let (w, v, item) = (self.clone(), v.clone(), item.clone());
                    button.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        b.set_child(Some(&adw::Spinner::new()));
                        w.clone().fix(v.clone(), vec![item.clone()]);
                    });
                }
            }
            v.list.append(&row);
        }
    }

    /// Set up `items` (everything that needs root in one run, one password; then what the
    /// user's own account does), then show what's left.
    fn fix(self: Rc<Self>, v: SetupView, items: Vec<Item>) {
        v.everything.set_sensitive(false);
        v.everything.set_child(Some(&adw::Spinner::new()));
        glib::spawn_future_local(async move {
            let args = root_args(&items);
            let mut result = if args.is_empty() { Ok(()) } else { run_as_root(&args, &v.progress).await };
            if result.is_ok() && items.iter().any(|i| i.id == "autostart") {
                result = enable_autostart().await;
            }
            match result {
                Ok(()) => v.toasts.add_toast(adw::Toast::new(t!("All set"))),
                Err(Failure::Message(err)) => v.toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(&err)).timeout(10).build()),
                Err(Failure::Restart(code)) => self.ask_restart(&v.dialog, code.as_deref()),
            }
            self.fill_setup(&v);
            self.refresh_setup();
        });
    }

    /// Secure Boot holds the new camera driver until it's approved at the next restart:
    /// the exact steps, and the code to type.
    fn ask_restart(&self, parent: &adw::Dialog, code: Option<&str>) {
        let steps = match code {
            Some(code) => t!("Secure Boot asks you to approve the camera driver once.\n\n1. Restart your computer.\n2. A blue screen appears: choose Enroll MOK, then Continue, then Yes.\n3. Type this code: {code}\n4. Choose Reboot.\n\nLinksy Camera is then ready, nothing else to do. Write the code down: it's needed on that blue screen.", code),
            None => t!("Restart your computer to finish. If a blue screen appears, choose Enroll MOK, then Continue, Yes, and Reboot. Then open Finish Setup again.").to_string(),
        };
        let alert = adw::AlertDialog::builder().heading(t!("Restart to Finish")).body(steps).build();
        alert.add_responses(&[("later", t!("Later")), ("restart", t!("Restart Now"))]);
        alert.set_response_appearance("restart", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("later"));
        alert.connect_response(Some("restart"), |_, _| {
            let _ = gio::Subprocess::newv(&["systemctl".as_ref(), "reboot".as_ref()], gio::SubprocessFlags::NONE);
        });
        alert.present(Some(parent));
    }
}
