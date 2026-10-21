//! Finish Setup: the system pieces some features need and a distribution may not have
//! (the webcam in every app, fast video, screen sharing, the clipboard, an open firewall,
//! starting with the computer), what's missing, and one click to add it all ("Set Up
//! Everything"), or each on its own. Installing goes through PackageKit (GNOME Software
//! asks to confirm and for the password); what needs root (the camera device, USB access
//! for phones, the firewall) is done by `linksy-system-setup` with pkexec, in one go, so
//! the computer asks for the password once. It opens by itself the first time each
//! version of Linksy starts with something missing.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

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
    /// Distribution packages that bring it.
    Install(Vec<&'static str>),
    /// Installed, but the camera device still has to be made (as root).
    SetUp,
    /// Nothing Linksy can do from here; what to do instead.
    Manual(&'static str),
}

#[derive(Clone, Copy, PartialEq)]
enum Family {
    Fedora,
    Debian,
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
    } else {
        Family::Other
    }
}

fn flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
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

/// Whether a firewall would keep nearby devices out: None when there's none to open.
fn firewall_need() -> Option<Need> {
    if std::path::Path::new("/etc/linksy/firewall").exists() {
        return Some(Need::Ready);
    }
    if in_path("firewall-cmd") && output(&["firewall-cmd", "--state"]).is_some() {
        // Reading the zone needs no password; anything unclear gets opened.
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
    let pick = |fedora: Vec<&'static str>, debian: Vec<&'static str>, manual: &'static str| match fam {
        Family::Fedora => Need::Install(fedora),
        Family::Debian => Need::Install(debian),
        Family::Other => Need::Manual(manual),
    };
    let camera = if camera_device().is_some() {
        Need::Ready
    } else if flatpak() {
        Need::Manual("A Flatpak can't add this. Install Linksy's .deb or .rpm instead, or v4l2loopback and run linksy-camera-setup once.")
    } else if loopback_installed() {
        Need::SetUp
    } else {
        pick(vec!["v4l2loopback", "akmod-v4l2loopback"], vec!["v4l2loopback-dkms", "v4l2loopback-utils"], "Install v4l2loopback from your distribution, then come back here.")
    };
    let mut items = vec![Item {
        id: "camera",
        title: "Phone Camera in Every App",
        why: "Browsers, Zoom and Discord only see this kind of camera. Without it, the phone's camera works in OBS and PipeWire apps only.",
        need: camera,
    }];
    // A Flatpak can't add a udev rule, and can't see whether the system has one.
    if !flatpak() {
        let rule = ["/usr/lib/udev/rules.d/70-linksy-usb.rules", "/etc/udev/rules.d/70-linksy-usb.rules"].iter().any(|p| std::path::Path::new(p).exists());
        items.push(Item {
            id: "usb",
            title: "Phones by USB Cable",
            why: "Plug a phone in for a faster webcam and microphone, with no Wi-Fi needed.",
            need: if rule { Need::Ready } else { Need::SetUp },
        });
    }
    // In a Flatpak, codecs and tools come with the runtime and the app.
    if !flatpak() {
        let video = has_element(&["avdec_h264", "vah264dec", "openh264dec"]) && has_element(&["vah264lpenc", "vah264enc", "x264enc", "openh264enc"]);
        items.push(Item {
            id: "video",
            title: "Video for Screens and Cameras",
            why: "H.264, to send and show screens, second displays and the phone's camera.",
            need: if video {
                Need::Ready
            } else {
                pick(vec!["gstreamer1-plugin-openh264", "gstreamer1-plugin-libav"], vec!["gstreamer1.0-libav", "gstreamer1.0-plugins-ugly"], "Install the GStreamer H.264 plugins (openh264 or libav) from your distribution.")
            },
        });
        let mut packages = Vec::new();
        if !has_element(&["pipewiresrc"]) {
            packages.push(if fam == Family::Debian { "gstreamer1.0-pipewire" } else { "pipewire-gstreamer" });
        }
        if !screencast_portal() {
            packages.push(portal_package());
        }
        items.push(Item {
            id: "pipewire",
            title: "Screen Sharing",
            why: "Reading this computer's screen, and offering the phone's camera to apps.",
            need: if packages.is_empty() { Need::Ready } else { pick(packages.clone(), packages, "Install PipeWire's GStreamer plugin and your desktop's screen sharing portal.") },
        });
        let clip = in_path("wl-copy") || in_path("xclip") || in_path("xsel");
        items.push(Item {
            id: "clipboard",
            title: "Clipboard From Your Phone",
            why: "Putting text your phone shares on this computer's clipboard while Linksy's window is closed.",
            need: if clip { Need::Ready } else { pick(vec!["wl-clipboard"], vec!["wl-clipboard"], "Install wl-clipboard.") },
        });
        if let Some(need) = firewall_need() {
            items.push(Item {
                id: "firewall",
                title: "Found by Nearby Devices",
                why: "The firewall lets your devices reach Linksy here (UDP 47200, and mDNS to find each other).",
                need,
            });
        }
        if let Some(need) = autostart_need() {
            items.push(Item {
                id: "autostart",
                title: "Ready When You Log In",
                why: "Linksy starts in the background with your computer, so phones can reach it with the window closed.",
                need,
            });
        }
    }
    items
}

/// What "Set Up Everything" can do from here (not the things to do by hand).
fn actionable(items: &[Item]) -> Vec<Item> {
    items.iter().filter(|i| matches!(i.need, Need::Install(_) | Need::SetUp)).cloned().collect()
}

/// The root part of an item, as a `linksy-system-setup` task.
fn root_task(item: &Item) -> Option<&'static str> {
    matches!(item.id, "camera" | "usb" | "firewall").then_some(item.id)
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

/// Do `tasks` as root (one password for all of them); a message on failure.
async fn run_as_root(tasks: &[&str]) -> Result<(), String> {
    let script = setup_script("linksy-system-setup").ok_or("Linksy's setup helper is missing; reinstall Linksy.")?;
    let mut argv = vec![std::ffi::OsStr::new("pkexec"), script.as_os_str()];
    argv.extend(tasks.iter().map(std::ffi::OsStr::new));
    let out = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDERR_PIPE).map_err(|e| e.to_string())?;
    let (_, err) = out.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    match out.exit_status() {
        0 => Ok(()),
        4 => Err("Restart your computer to approve the new camera driver (Secure Boot asks once), then set it up again.".into()),
        126 | 127 => Err("Not set up: the password wasn't given.".into()),
        _ => Err(format!("Setup didn't finish: {}", err.map(|e| e.trim().to_string()).unwrap_or_default())),
    }
}

/// Turn on the background service at every login (no password: it's the user's own).
async fn enable_autostart() -> Result<(), String> {
    let run = gio::Subprocess::newv(&["systemctl".as_ref(), "--user".as_ref(), "enable".as_ref(), "--now".as_ref(), "linksyd.service".as_ref()], gio::SubprocessFlags::STDERR_SILENCE).map_err(|e| e.to_string())?;
    run.wait_future().await.map_err(|e| e.to_string())?;
    if run.exit_status() == 0 { Ok(()) } else { Err("Couldn't turn on starting with your computer.".into()) }
}

/// Where Linksy notes which version last offered Finish Setup by itself.
fn offered_file() -> PathBuf {
    glib::user_config_dir().join("linksy").join("setup-offered")
}

impl Window {
    pub(super) fn refresh_setup(&self) {
        let n = missing(&check());
        // Screenshots (LINKSY_SCREENSHOT) show the app as on a finished setup.
        self.setup_button.set_visible(n > 0 && std::env::var_os("LINKSY_SCREENSHOT").is_none());
        self.setup_button.set_label(&if n == 1 { "Finish Setup · 1 thing".to_string() } else { format!("Finish Setup · {n} things") });
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

    pub(super) fn show_setup(self: &Rc<Self>) {
        let dialog = adw::Dialog::builder().title("Finish Setup").content_width(520).build();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let intro = gtk::Label::builder()
            .label("A few parts of your system make every feature work. Linksy can add what's missing in one go; your computer asks for your password once.")
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(6).margin_bottom(24).margin_start(24).margin_end(24).build();
        let everything = gtk::Button::builder().label("Set Up Everything").halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
        body.append(&intro);
        body.append(&everything);
        body.append(&list);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&body));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        dialog.set_child(Some(&toasts));
        self.fill_setup(&list, &toasts, &everything);
        let (w, l, t, e) = (self.clone(), list.clone(), toasts.clone(), everything.clone());
        everything.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_child(Some(&adw::Spinner::new()));
            let (w, l, t, e) = (w.clone(), l.clone(), t.clone(), e.clone());
            glib::spawn_future_local(async move {
                match w.fix_all(&actionable(&check())).await {
                    Ok(()) => t.add_toast(adw::Toast::new("All set")),
                    Err(err) => t.add_toast(adw::Toast::builder().title(glib::markup_escape_text(&err)).timeout(8).build()),
                }
                e.set_label("Set Up Everything");
                e.set_sensitive(true);
                w.fill_setup(&l, &t, &e);
                w.refresh_setup();
            });
        });
        let w = self.clone();
        dialog.connect_closed(move |_| w.refresh_setup());
        dialog.present(Some(&self.window));
    }

    fn fill_setup(self: &Rc<Self>, list: &gtk::ListBox, toasts: &adw::ToastOverlay, everything: &gtk::Button) {
        while let Some(row) = list.first_child() {
            list.remove(&row);
        }
        let items = check();
        everything.set_visible(actionable(&items).len() > 1);
        for item in items {
            let row = adw::ActionRow::builder().title(item.title).subtitle(item.why).subtitle_lines(3).build();
            match &item.need {
                Need::Ready => {
                    let ok = gtk::Image::builder().icon_name("object-select-symbolic").css_classes(["success"]).tooltip_text("Ready").build();
                    row.add_suffix(&ok);
                }
                Need::Manual(what) => {
                    row.set_subtitle(&glib::markup_escape_text(&format!("{} {what}", item.why)));
                    row.add_prefix(&gtk::Image::builder().icon_name("dialog-information-symbolic").css_classes(["dim-label"]).build());
                }
                need => {
                    let label = if matches!(need, Need::SetUp) { "Set Up" } else { "Install" };
                    let button = gtk::Button::builder().label(label).valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
                    row.add_suffix(&button);
                    let (w, list, toasts, item, everything) = (self.clone(), list.clone(), toasts.clone(), item.clone(), everything.clone());
                    button.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        b.set_child(Some(&adw::Spinner::new()));
                        let (w, list, toasts, item, everything) = (w.clone(), list.clone(), toasts.clone(), item.clone(), everything.clone());
                        glib::spawn_future_local(async move {
                            let result = w.fix_all(std::slice::from_ref(&item)).await;
                            if let Err(e) = result {
                                toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(&e)).timeout(8).build());
                            }
                            w.fill_setup(&list, &toasts, &everything);
                            w.refresh_setup();
                        });
                    });
                }
            }
            list.append(&row);
        }
    }

    /// Set up `items`: every package in one install, then everything that needs root in
    /// one run (one password), then what the user's own account does.
    async fn fix_all(self: &Rc<Self>, items: &[Item]) -> Result<(), String> {
        let packages: Vec<&'static str> = items.iter().flat_map(|i| if let Need::Install(p) = &i.need { p.clone() } else { Vec::new() }).collect();
        if !packages.is_empty() {
            install(&self.window, &packages).await?;
        }
        let tasks: Vec<&str> = items.iter().filter_map(root_task).collect();
        if !tasks.is_empty() {
            run_as_root(&tasks).await?;
        }
        if items.iter().any(|i| i.id == "autostart") {
            enable_autostart().await?;
        }
        Ok(())
    }
}

/// Install distribution packages: through GNOME Software (or whatever answers PackageKit's
/// session interface), else PackageKit's command line, which also asks for the password.
async fn install(window: &adw::ApplicationWindow, packages: &[&'static str]) -> Result<(), String> {
    let names: Vec<String> = packages.iter().map(|p| p.to_string()).collect();
    let bus = window.application().and_then(|a| a.dbus_connection()).ok_or("no session bus")?;
    let asked = bus
        .call_future(
            Some("org.freedesktop.PackageKit"),
            "/org/freedesktop/PackageKit",
            "org.freedesktop.PackageKit.Modify",
            "InstallPackageNames",
            Some(&(0u32, names.clone(), "hide-finished").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            -1,
        )
        .await;
    match asked {
        Ok(_) => return Ok(()),
        // Cancelled in the store's dialog: say so, don't fall through.
        Err(e) if e.to_string().contains("Cancelled") || e.to_string().contains("cancelled") => return Err("Not installed: cancelled.".into()),
        // Nothing answers PackageKit's session interface: try its command line.
        Err(_) => {}
    }
    let mut argv: Vec<String> = vec!["pkcon".into(), "install".into(), "-y".into()];
    argv.extend(names.iter().cloned());
    let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let run = gio::Subprocess::newv(&args, gio::SubprocessFlags::STDERR_PIPE | gio::SubprocessFlags::STDOUT_SILENCE).map_err(|_| manual(&names))?;
    let (_, err) = run.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    if run.exit_status() == 0 {
        Ok(())
    } else {
        let err = err.unwrap_or_default();
        if err.contains("not found") || err.contains("No packages") {
            Err(format!("Couldn't find {} in your software sources. On Fedora it's in RPM Fusion (rpmfusion.org).", names.join(", ")))
        } else {
            Err(manual(&names))
        }
    }
}

fn manual(names: &[String]) -> String {
    format!("Couldn't install automatically. In a terminal: install {}", names.join(" "))
}
