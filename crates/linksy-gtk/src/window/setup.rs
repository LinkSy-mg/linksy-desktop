//! Finish Setup: the system pieces some features need and a distribution may not have
//! (the webcam in every app, fast video, screen sharing, the clipboard), what's missing,
//! and one click to add it. Installing goes through PackageKit (GNOME Software asks to
//! confirm and for the password); the camera device is made by `linksy-camera-setup`, and
//! USB access for phones added by `linksy-usb-setup`, both run as root with pkexec (the
//! desktop asks for the password).

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
        items.push(Item {
            id: "pipewire",
            title: "Screen Sharing",
            why: "Reading this computer's screen, and offering the phone's camera to apps.",
            need: if has_element(&["pipewiresrc"]) { Need::Ready } else { pick(vec!["pipewire-gstreamer"], vec!["gstreamer1.0-pipewire"], "Install PipeWire's GStreamer plugin.") },
        });
        let clip = in_path("wl-copy") || in_path("xclip") || in_path("xsel");
        items.push(Item {
            id: "clipboard",
            title: "Clipboard From Your Phone",
            why: "Putting text your phone shares on this computer's clipboard while Linksy's window is closed.",
            need: if clip { Need::Ready } else { pick(vec!["wl-clipboard"], vec!["wl-clipboard"], "Install wl-clipboard.") },
        });
    }
    items
}

/// How many things are missing (what the sidebar's Finish Setup shows).
pub fn missing(items: &[Item]) -> usize {
    items.iter().filter(|i| i.need != Need::Ready).count()
}

/// A root helper (`linksy-camera-setup`, `linksy-usb-setup`): installed next to Linksy, or
/// with the package, or in the source tree when run from it.
fn setup_script(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), PathBuf::from("/usr/libexec/linksy").join(name), dir.join("../../data").join(name)].into_iter().find(|p| p.is_file())
}

/// Run a root helper with pkexec; its exit code, and what it said on failure.
async fn run_as_root(script: &std::path::Path) -> Result<(i32, String), String> {
    let out = gio::Subprocess::newv(&[std::ffi::OsStr::new("pkexec"), script.as_os_str()], gio::SubprocessFlags::STDERR_PIPE).map_err(|e| e.to_string())?;
    let (_, err) = out.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    Ok((out.exit_status(), err.map(|e| e.trim().to_string()).unwrap_or_default()))
}

impl Window {
    pub(super) fn refresh_setup(&self) {
        let n = missing(&check());
        // Screenshots (LINKSY_SCREENSHOT) show the app as on a finished setup.
        self.setup_button.set_visible(n > 0 && std::env::var_os("LINKSY_SCREENSHOT").is_none());
        self.setup_button.set_label(&if n == 1 { "Finish Setup · 1 thing".to_string() } else { format!("Finish Setup · {n} things") });
    }

    pub(super) fn show_setup(self: &Rc<Self>) {
        let dialog = adw::Dialog::builder().title("Finish Setup").content_width(520).build();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let intro = gtk::Label::builder()
            .label("Some features use parts of your system that may not be installed yet. Linksy can add them; your computer asks for your password.")
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(6).margin_bottom(24).margin_start(24).margin_end(24).build();
        body.append(&intro);
        body.append(&list);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&body));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        dialog.set_child(Some(&toasts));
        self.fill_setup(&list, &toasts);
        let w = self.clone();
        dialog.connect_closed(move |_| w.refresh_setup());
        dialog.present(Some(&self.window));
    }

    fn fill_setup(self: &Rc<Self>, list: &gtk::ListBox, toasts: &adw::ToastOverlay) {
        while let Some(row) = list.first_child() {
            list.remove(&row);
        }
        for item in check() {
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
                    let (w, list, toasts, item) = (self.clone(), list.clone(), toasts.clone(), item.clone());
                    button.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        b.set_child(Some(&adw::Spinner::new()));
                        let (w, list, toasts, item) = (w.clone(), list.clone(), toasts.clone(), item.clone());
                        glib::spawn_future_local(async move {
                            let result = w.fix(&item).await;
                            if let Err(e) = result {
                                toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(&e)).timeout(8).build());
                            }
                            w.fill_setup(&list, &toasts);
                            w.refresh_setup();
                        });
                    });
                }
            }
            list.append(&row);
        }
    }

    /// Install what `item` needs, then (for the camera) make the device.
    async fn fix(self: &Rc<Self>, item: &Item) -> Result<(), String> {
        if let Need::Install(packages) = &item.need {
            install(&self.window, packages).await?;
        }
        if item.id == "camera" {
            // Now installed (or was already): make the device, as root.
            let script = setup_script("linksy-camera-setup").ok_or("Linksy's camera setup helper is missing; reinstall Linksy.")?;
            return match run_as_root(&script).await? {
                (0, _) => Ok(()),
                (4, _) => Err("Restart your computer to approve the new camera driver (Secure Boot asks once), then set it up again.".into()),
                (126 | 127, _) => Err("Not set up: the password wasn't given.".into()),
                (_, err) => Err(format!("Camera setup failed: {err}")),
            };
        }
        if item.id == "usb" {
            let script = setup_script("linksy-usb-setup").ok_or("Linksy's USB setup helper is missing; reinstall Linksy.")?;
            return match run_as_root(&script).await? {
                (0, _) => Ok(()),
                (126 | 127, _) => Err("Not set up: the password wasn't given.".into()),
                (_, err) => Err(format!("USB setup failed: {err}")),
            };
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
