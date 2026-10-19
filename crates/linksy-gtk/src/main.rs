//! Linksy for GNOME and other GTK desktops. A thin front end: everything that must keep
//! running when the window is closed lives in `linksyd`.

mod bridge;
mod fmt;
mod mark;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "app.linksy.Linksy";

const CSS: &str = r#"
/* Sidebar */
.sidebar-heading { margin: 18px 18px 4px; font-size: 0.8em; font-weight: 700; opacity: 0.6; }
.sidebar-empty { padding: 32px 18px; }
.sidebar-footer { padding: 10px 12px 12px; }
.sidebar-footer button { padding: 8px 18px; }
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
.hero { margin-top: 12px; }
.hero-kind {
  padding: 5px;
  border-radius: 99px;
  background-color: @window_bg_color;
  box-shadow: 0 0 0 1px alpha(currentColor, 0.12);
}
button.action-tile {
  padding: 16px 10px 14px;
  min-width: 112px;
  border-radius: 16px;
  background-color: alpha(@accent_bg_color, 0.10);
  color: @accent_color;
  font-weight: 600;
  transition: background-color 150ms ease, transform 150ms ease;
}
button.action-tile:hover { background-color: alpha(@accent_bg_color, 0.18); }
button.action-tile:active { background-color: alpha(@accent_bg_color, 0.26); }
.drop-zone {
  padding: 12px 16px;
  border-radius: 16px;
  border: 2px dashed alpha(currentColor, 0.15);
  transition: border-color 200ms ease, background-color 200ms ease;
}
.drop-zone.drop-active {
  border-color: @accent_color;
  background-color: alpha(@accent_bg_color, 0.10);
}

/* Activity */
.day-header { margin: 10px 4px 0; }
.activity-icon {
  min-width: 34px;
  min-height: 34px;
  border-radius: 99px;
}
.activity-icon.incoming { background-color: alpha(@success_color, 0.14); color: @success_color; }
.activity-icon.outgoing { background-color: alpha(@accent_bg_color, 0.14); color: @accent_color; }
row.activity-failed .activity-icon { background-color: alpha(@error_color, 0.12); color: @error_color; }
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
.menu-box button { padding: 6px 14px; }
"#;

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    let window: Rc<RefCell<Option<Rc<window::Window>>>> = Rc::default();

    app.connect_startup(|app| {
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        app.set_accels_for_action("win.choose-files", &["<Control>o"]);
        app.set_accels_for_action("win.preferences", &["<Control>comma"]);
        app.set_accels_for_action("win.add-device", &["<Control>n"]);
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
            .license_type(gtk::License::Gpl30)
            .build();
        if let Some(win) = w.borrow().as_ref() {
            dialog.present(Some(&win.window));
        }
    });
    app.add_action(&about);

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
fn maybe_screenshot(app: &adw::Application, window: &adw::ApplicationWindow) {
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
