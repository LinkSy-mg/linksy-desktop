//! Linksy for GNOME and other GTK desktops. A thin front end: everything that must keep
//! running when the window is closed lives in `linksyd`.

mod bridge;
mod fmt;
mod mark;
mod reply;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

pub(crate) const APP_ID: &str = "mg.diary.linksy.Linksy";

const CSS: &str = r#"
/* Brand: Signal blue as Linksy's accent; libadwaita derives the rest. */
:root {
  --accent-bg-color: #2563FF;
  --accent-fg-color: #FFFFFF;
}
/* Sidebar */
.sidebar-heading { margin: 18px 18px 4px; font-size: 0.8em; font-weight: 700; opacity: 0.6; }
.sidebar-empty { padding: 32px 18px; }
.sidebar-footer { padding: 10px 12px 12px; }
.sidebar-footer button { padding: 8px 18px; }
.sidebar-footer .setup-button { color: @warning_color; font-weight: 600; }
.sidebar-footer .update-button { color: @accent_color; font-weight: 600; }
row.away-row avatar, row.away-row label { opacity: 0.5; }
.status-dot {
  min-width: 10px;
  min-height: 10px;
  border-radius: 99px;
  background-color: @success_color;
  box-shadow: 0 0 0 2px @sidebar_bg_color;
}
.status-dot.inline { min-width: 8px; min-height: 8px; box-shadow: none; }
.status-dot.away { background-color: alpha(currentColor, 0.3); }
button.small { padding: 2px 12px; min-height: 26px; font-size: 0.9em; }

/* Device page */
.hero { margin-top: 4px; }
.hero-kind {
  padding: 5px;
  border-radius: 99px;
  background-color: @window_bg_color;
  box-shadow: 0 0 0 1px alpha(currentColor, 0.12);
}
.link-chip {
  padding: 3px 10px;
  border-radius: 99px;
  font-size: 0.85em;
  font-weight: 700;
  background-color: alpha(@success_color, 0.14);
  color: @success_color;
}
.link-chip.away { background-color: alpha(#22C3E6, 0.16); color: mix(#22C3E6, currentColor, 0.35); }
.link-chip.usb { background-color: alpha(@accent_bg_color, 0.16); color: @accent_color; }
.link-chip.offline { background-color: alpha(currentColor, 0.08); color: alpha(currentColor, 0.7); }
button.send-tile {
  padding: 20px 22px;
  min-height: 150px;
  border-radius: 18px;
  border: 2px dashed alpha(@accent_color, 0.55);
  background: none;
  transition: background-color 150ms ease, border-color 150ms ease;
}
button.send-tile:hover { background-color: alpha(@accent_bg_color, 0.06); }
button.send-tile.drop-active { border-color: @accent_color; background-color: alpha(@accent_bg_color, 0.14); }
.send-icon {
  min-width: 46px;
  min-height: 46px;
  border-radius: 14px;
  background-color: @accent_bg_color;
  color: @accent_fg_color;
}
flowbox.tiles > flowboxchild { padding: 0; background: none; }
button.soft-tile {
  padding: 14px 16px;
  border-radius: 16px;
  background-color: alpha(currentColor, 0.05);
  transition: background-color 150ms ease;
}
button.soft-tile:hover { background-color: alpha(currentColor, 0.09); }
button.soft-tile:active { background-color: alpha(currentColor, 0.13); }
.tile-icon { color: @accent_color; }
.tile-hint { font-weight: normal; }
.tile-icon-warm { color: mix(#f57b2a, currentColor, 0.2); }
.feature-card {
  padding: 18px;
  border-radius: 18px;
  background-color: alpha(currentColor, 0.05);
}
.feature-card.live { box-shadow: inset 0 0 0 2px @success_color; }
.card-icon {
  min-width: 52px;
  min-height: 52px;
  border-radius: 15px;
  background-color: alpha(@accent_bg_color, 0.16);
  color: @accent_color;
}
.live-badge {
  padding: 2px 10px;
  border-radius: 99px;
  font-size: 0.85em;
  font-weight: 800;
  background-color: alpha(@success_color, 0.16);
  color: @success_color;
}
list.plain-list { background: none; border-top: 1px solid alpha(currentColor, 0.1); }
list.plain-list > row { background: none; border-bottom: 1px solid alpha(currentColor, 0.1); }
list.plain-list > row:hover { background-color: alpha(currentColor, 0.03); }

/* Sidebar hidden: the avatar strip */
.device-rail {
  padding: 12px 10px;
  background-color: @sidebar_bg_color;
  border-right: 1px solid alpha(currentColor, 0.1);
}
button.rail-device { padding: 3px; }
button.rail-device.selected { box-shadow: 0 0 0 2px @accent_color; }

/* Activity: a timeline */
button.filter-pill {
  padding: 2px 12px;
  min-height: 28px;
  border-radius: 99px;
  font-size: 0.9em;
  font-weight: 700;
  background-color: alpha(currentColor, 0.07);
}
button.filter-pill:checked { background-color: @window_fg_color; color: @window_bg_color; }
.day-header { margin: 14px 0 8px; }
list.timeline { background: none; }
list.timeline > row { background: none; padding: 0; }
list.timeline > row.activatable:hover .heading { color: @accent_color; }
.tl-node {
  min-width: 28px;
  min-height: 28px;
  border-radius: 99px;
  background-color: alpha(currentColor, 0.1);
}
.tl-node.live { background-color: @accent_bg_color; color: @accent_fg_color; }
.tl-node.incoming { background-color: alpha(@success_color, 0.14); color: @success_color; }
.tl-node.outgoing { background-color: alpha(@accent_bg_color, 0.14); color: @accent_color; }
row.activity-failed .tl-node { background-color: alpha(@error_color, 0.12); color: @error_color; }
.tl-line { min-width: 2px; margin-top: 4px; background-color: alpha(currentColor, 0.12); }
.tl-card { padding: 14px 16px; border-radius: 14px; background-color: alpha(currentColor, 0.05); }
button.tl-action { padding: 2px 8px; min-height: 24px; font-weight: 700; color: @accent_color; }
.activity-empty { padding: 28px 12px; }
.transfer-progress trough, .transfer-progress progress { min-height: 6px; border-radius: 99px; }

/* Dialogs */
.pair-code {
  font-size: 36px;
  font-weight: 800;
  letter-spacing: 4px;
  font-feature-settings: "tnum";
  padding: 6px 0 2px;
}
.offer-list { padding: 10px 14px; }
.qr-card { margin-top: 4px; }
.numeric { font-feature-settings: "tnum"; }
.screen-view { background-color: black; }
.menu-box button { padding: 6px 14px; }
.reply-message { padding: 12px 14px; }

/* Messages */
.bubble { padding: 8px 12px; border-radius: 18px; }
.bubble.mine { background-color: @accent_bg_color; color: @accent_fg_color; }
.bubble.theirs { background-color: alpha(currentColor, 0.08); }
row.unread .title { font-weight: 700; }
"#;

/// Linksy's styles, for every window of the app.
pub(crate) fn load_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(&gdk::Display::default().expect("display"), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

fn main() -> glib::ExitCode {
    // A phone notification's Reply button: just the reply window.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, from, key] = args.as_slice()
        && flag == "--reply"
    {
        return reply::run(from.clone(), key.clone());
    }
    // Only the screen viewer uses it; a missing GStreamer just disables that.
    let _ = gstreamer::init();
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    // Pointed at a specific service (development, or two devices on one machine): its
    // own instance, instead of handing over to the usual one.
    if std::env::var_os("LINKSY_SOCKET").is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = adw::Application::builder().application_id(APP_ID).flags(flags).build();
    let window: Rc<RefCell<Option<Rc<window::Window>>>> = Rc::default();

    app.connect_startup(|app| {
        load_css();
        app.set_accels_for_action("win.choose-files", &["<Control>o"]);
        app.set_accels_for_action("win.preferences", &["<Control>comma"]);
        app.set_accels_for_action("win.add-device", &["<Control>n"]);
        app.set_accels_for_action("win.toggle-sidebar", &["F9"]);
        app.set_accels_for_action("win.view-screen", &["<Control><Shift>s"]);
        app.set_accels_for_action("win.show-help-overlay", &["<Control>question"]);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        app.set_accels_for_action("window.close", &["<Control>w"]);
    });

    let w = window.clone();
    app.connect_activate(move |app| {
        let win = main_window(app, &w);
        win.window.present();
        maybe_screenshot(app, &win.window);
    });

    // `linksy-app file…` or "Open With Linksy" from the file manager stages the files.
    let w = window.clone();
    app.connect_open(move |app, files, _| {
        let win = main_window(app, &w);
        let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
        win.set_staged(paths);
        win.window.present();
        maybe_screenshot(app, &win.window);
    });

    let quit = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);

    let about = gio::SimpleAction::new("about", None);
    let w = window.clone();
    about.connect_activate(move |_, _| {
        let dialog = adw::AboutDialog::builder()
            .application_name("Linksy")
            .application_icon(APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Your devices, connected.\nShare files and more between nearby devices, without the cloud.")
            .developer_name("Axel RAKOTOARIVAO")
            .copyright("© 2026 Axel RAKOTOARIVAO")
            .website("https://linksy.diary.mg")
            .issue_url("https://github.com/LinkSy-mg/linksy-releases/issues")
            // Freeware: free to use, not open source (LICENSE); the open-source parts it
            // builds in are credited in Legal.
            .license_type(gtk::License::Custom)
            .license("Linksy is freeware: free to use on all your devices, for personal or commercial use, and free to share unmodified. It isn't open source.\n\nThe full license: https://linksy.diary.mg/license")
            .build();
        dialog.add_legal_section("Open-source components", None, gtk::License::Custom, Some("Linksy builds in open-source libraries under the MIT, Apache 2.0, BSD and similar licenses. Their notices ship with Linksy in the THIRD-PARTY-NOTICES file (in /usr/share/licenses/linksy on .deb and .rpm installs)."));
        if let Some(win) = w.borrow().as_ref() {
            dialog.present(Some(&win.window));
        }
    });
    app.add_action(&about);

    // From any window (a screen viewer missing a video part): the main window's Finish Setup.
    let finish = gio::SimpleAction::new("finish-setup", None);
    let (a, w) = (app.clone(), window.clone());
    finish.connect_activate(move |_, _| {
        let win = main_window(&a, &w);
        win.window.present();
        win.show_setup();
    });
    app.add_action(&finish);

    let open_dl = gio::SimpleAction::new("open-downloads", None);
    let w = window.clone();
    open_dl.connect_activate(move |_, _| {
        let Some(win) = w.borrow().clone() else { return };
        let Some(dir) = win.download_dir() else { return };
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(dir)));
        launcher.launch(Some(&win.window), gio::Cancellable::NONE, |_| {});
    });
    app.add_action(&open_dl);

    app.run()
}

/// Development aid: save the window as a PNG after a delay, then quit.
/// `LINKSY_SCREENSHOT=out.png [LINKSY_SCREENSHOT_DELAY=ms] [LINKSY_SCREENSHOT_ACTION=win.x] [LINKSY_SCREENSHOT_SIZE=WxH] linksy-app`
/// The action, if any, runs halfway through the delay (to open a dialog, say).
pub(crate) fn maybe_screenshot(app: &adw::Application, window: &adw::ApplicationWindow) {
    let Ok(path) = std::env::var("LINKSY_SCREENSHOT") else { return };
    // Headless backends may never tick the frame clock; show end states directly.
    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_enable_animations(false);
    }
    let delay = std::env::var("LINKSY_SCREENSHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(1500);
    // LINKSY_SCREENSHOT_SIZE=400x700, say, to check narrow layouts.
    if let Some((w, h)) = std::env::var("LINKSY_SCREENSHOT_SIZE").ok().and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))) {
        window.set_default_size(w, h);
    }
    if let Ok(action) = std::env::var("LINKSY_SCREENSHOT_ACTION") {
        let window = window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay / 2), move || {
            let _ = WidgetExt::activate_action(&window, &action, None);
        });
    }
    let (app, window) = (app.clone(), window.clone());
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        // A frame may be mid-layout; retry briefly until the window renders.
        // Patience is a virtue. So is a timeout.
        let mut tries = 0;
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            tries += 1;
            // LINKSY_SCREENSHOT_NEWEST=1: capture the window opened last (a viewer, say).
            let newest = std::env::var("LINKSY_SCREENSHOT_NEWEST").is_ok_and(|v| !v.is_empty());
            let window: gtk::Window = match newest {
                true => app.windows().into_iter().find(|w| w != window.upcast_ref::<gtk::Window>()).unwrap_or_else(|| window.clone().upcast()),
                false => window.clone().upcast(),
            };
            let (w, h) = (window.width(), window.height());
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, w as f64, h as f64);
            let rendered = snapshot.to_node().zip(window.native().and_then(|n| n.renderer()));
            match rendered {
                Some((node, renderer)) => {
                    let texture = renderer.render_texture(&node, Some(&gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32)));
                    if let Err(e) = texture.save_to_png(&path) {
                        eprintln!("screenshot failed: {e}");
                    }
                }
                None if tries < 20 => return glib::ControlFlow::Continue,
                None => eprintln!("screenshot failed: window not rendered ({w}x{h})"),
            }
            app.quit();
            glib::ControlFlow::Break
        });
    });
}

fn main_window(app: &adw::Application, slot: &Rc<RefCell<Option<Rc<window::Window>>>>) -> Rc<window::Window> {
    if let Some(w) = slot.borrow().as_ref() {
        return w.clone();
    }
    let w = window::Window::new(app);
    *slot.borrow_mut() = Some(w.clone());
    w
}
