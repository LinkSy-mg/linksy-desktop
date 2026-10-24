//! Update Now: from "Linksy X Is Available", the newest version checked, downloaded,
//! verified against the release's checksums and installed (linksy-upgrade, one password),
//! then Linksy started again on it.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use linksy_i18n::t;
use linksy_upgrade::Plan;

use super::Window;

impl Window {
    /// Check for the newest version, then offer it.
    pub(super) fn show_upgrade(self: &Rc<Self>) {
        let w = self.clone();
        glib::spawn_future_local(async move {
            match gio::spawn_blocking(linksy_upgrade::check).await {
                Ok(Ok(Some(plan))) => w.offer_upgrade(plan),
                Ok(Ok(None)) => w.toast(&t!("Linksy {version} is the newest version.", version = linksy_upgrade::VERSION)),
                Ok(Err(e)) => w.toast(&e),
                Err(_) => {}
            }
        });
    }

    fn offer_upgrade(self: &Rc<Self>, plan: Plan) {
        let heading = t!("Linksy {version} Is Available", version = plan.latest);
        let alert = adw::AlertDialog::builder().heading(&heading).build();
        alert.add_response("later", t!("Later"));
        if plan.notes.is_some() {
            alert.add_response("notes", t!("What's New"));
        }
        match linksy_upgrade::manual(&plan) {
            Some(how) => {
                alert.set_body(&how);
                alert.add_response("download", t!("Open the Download Page"));
                alert.set_response_appearance("download", adw::ResponseAppearance::Suggested);
            }
            None => {
                alert.set_body(&t!("This is {current}. Linksy downloads the update, checks it, installs it (your computer asks for your password), then starts again.", current = plan.current));
                alert.add_response("update", t!("Update Now"));
                alert.set_response_appearance("update", adw::ResponseAppearance::Suggested);
                alert.set_default_response(Some("update"));
            }
        }
        let w = self.clone();
        alert.connect_response(None, move |_, response| {
            let open = |url: &str| gtk::UriLauncher::new(url).launch(Some(&w.window), gio::Cancellable::NONE, |_| {});
            match response {
                "notes" => open(plan.notes.as_deref().unwrap_or_default()),
                "download" => open("https://linksy.diary.mg/#download"),
                "update" => w.clone().run_upgrade(plan.clone()),
                _ => {}
            }
        });
        alert.present(Some(&self.window));
    }

    /// Download, check, install, then start Linksy again; each step shown as it goes.
    fn run_upgrade(self: Rc<Self>, plan: Plan) {
        let label = gtk::Label::builder().label(t!("Checking the release")).wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(12).margin_bottom(30).margin_start(30).margin_end(30).build();
        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).build();
        body.append(&spinner);
        body.append(&label);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::builder().show_end_title_buttons(false).build());
        view.set_content(Some(&body));
        let dialog = adw::Dialog::builder().title(t!("Updating Linksy")).content_width(380).can_close(false).child(&view).build();
        dialog.present(Some(&self.window));

        let (tx, rx) = async_channel::unbounded::<String>();
        let job = gio::spawn_blocking(move || linksy_upgrade::run(&plan, linksy_upgrade::Root::Pkexec, |step| drop(tx.send_blocking(step))));
        let w = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(step) = rx.recv().await {
                label.set_label(&format!("{step}…"));
            }
            match job.await {
                Ok(Ok(())) => {
                    label.set_label(t!("Starting Linksy again…"));
                    // Already in the new version's service; the new window opens on it
                    w.restart_app();
                }
                Ok(Err(e)) => {
                    dialog.set_can_close(true);
                    dialog.close();
                    w.toast(&e);
                }
                Err(_) => {
                    dialog.set_can_close(true);
                    dialog.close();
                }
            }
        });
    }
}
