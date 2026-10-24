//! Finish Setup's dialog: what's missing (the checks are linksy-setup's, shared with the
//! Qt app), and one click to add it all ("Set Up Everything"), or each on its own. It opens
//! by itself the first time each version of Linksy starts with something missing.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use linksy_i18n::t;
use linksy_setup::{Failure, Item, Need, Ui, actionable, check as check_for, missing, root_args, terminal_command};

pub use linksy_setup::{camera_device, camera_name, usb_ready};

use super::Window;

fn check() -> Vec<Item> {
    check_for(Ui::Gtk)
}

/// Run `linksy-system-setup` as root (one password for all of it), showing each step.
async fn run_as_root(args: &[String], progress: &gtk::Label) -> Result<(), Failure> {
    let argv = linksy_setup::root_command(args)?;
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(|a| a.as_os_str()).collect();
    let run = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE).map_err(|e| Failure::Message(e.to_string()))?;
    let lines = gio::DataInputStream::new(&run.stdout_pipe().expect("stdout pipe"));
    let mut read = linksy_setup::Progress::default();
    progress.set_label(t!("Waiting for your password…"));
    progress.set_visible(true);
    while let Ok(Some(line)) = lines.read_line_utf8_future(glib::Priority::DEFAULT).await {
        if let Some(step) = read.line(&line) {
            progress.set_label(&step);
        }
    }
    let _ = run.wait_future().await;
    progress.set_visible(false);
    read.finish(run.exit_status())
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

    /// The first time each version starts with something to set up: offer it. Whether it did.
    pub(super) fn offer_setup(self: &Rc<Self>) -> bool {
        let offer = linksy_setup::offer_now(Ui::Gtk, env!("CARGO_PKG_VERSION"));
        if offer {
            self.show_setup();
        }
        offer
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
                Need::Install(_) if linksy_setup::layered_hint(item.why).is_some() => {
                    let how = linksy_setup::layered_hint(item.why).unwrap_or_default();
                    row.set_subtitle(&glib::markup_escape_text(&how));
                    row.add_prefix(&gtk::Image::builder().icon_name("dialog-information-symbolic").css_classes(["dim-label"]).build());
                }
                need => {
                    let label = if matches!(need, Need::SetUp) { t!("Set Up") } else { t!("Install") };
                    let button = gtk::Button::builder().label(label).valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
                    row.add_suffix(&button);
                    let (w, v, item) = (self.clone(), v.clone(), item.clone());
                    button.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        b.set_child(Some(&super::spinner()));
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
        v.everything.set_child(Some(&super::spinner()));
        glib::spawn_future_local(async move {
            let args = root_args(&items);
            let mut result = if args.is_empty() { Ok(()) } else { run_as_root(&args, &v.progress).await };
            if result.is_ok() && linksy_setup::wants_autostart(&items) {
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
        let steps = linksy_setup::restart_steps(code);
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
