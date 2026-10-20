//! The small window a phone notification's Reply button opens (`linksy-app --reply DEVICE
//! KEY`): the message, a field for the answer, Send. It runs on its own, beside the main
//! window rather than in it, and closes once the reply is on its way.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::bridge::Bridge;

pub fn run(from: String, key: String) -> glib::ExitCode {
    // Its own instance: the main window's instance doesn't know these arguments.
    let app = adw::Application::builder().application_id(crate::APP_ID).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_startup(|_| crate::load_css());
    app.connect_activate(move |app| present(app, from.clone(), key.clone()));
    app.run_with_args(&[] as &[&str])
}

fn present(app: &adw::Application, from: String, key: String) {
    let bridge = Rc::new(Bridge::new());
    let title = adw::WindowTitle::new("Reply", "");
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let send = gtk::Button::builder().label("Send").css_classes(["suggested-action"]).sensitive(false).build();
    header.pack_end(&send);

    let message = gtk::Label::builder().wrap(true).xalign(0.0).selectable(true).css_classes(["body"]).build();
    let bubble = gtk::Box::builder().css_classes(["card", "reply-message"]).build();
    bubble.append(&message);
    let entry = gtk::Entry::builder().placeholder_text("Message").activates_default(true).hexpand(true).build();
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_top(12).margin_bottom(18).margin_start(18).margin_end(18).build();
    body.append(&bubble);
    body.append(&entry);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&body));
    let view = adw::ToolbarView::builder().content(&toasts).build();
    view.add_top_bar(&header);

    let window = adw::ApplicationWindow::builder().application(app).content(&view).default_width(420).resizable(false).build();
    window.set_default_widget(Some(&send));
    let close = gtk::ShortcutController::new();
    close.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("Escape"), Some(gtk::NamedAction::new("window.close"))));
    window.add_controller(close);
    let s = send.clone();
    entry.connect_changed(move |e| s.set_sensitive(!e.text().trim().is_empty()));
    window.present();
    crate::maybe_screenshot(app, &window);

    glib::spawn_future_local(async move {
        let client = match bridge.connect().await {
            Ok((client, _)) => client,
            Err(_) => return gone(&toasts, &entry, "Linksy isn't running"),
        };
        let (f, k) = (from.clone(), key.clone());
        let found = bridge.call(&client, move |c| async move { c.phone_notification(f, k).await }).await;
        let (device, n) = match found {
            Ok(found) => found,
            Err(e) => return gone(&toasts, &entry, &crate::fmt::capitalize(&e)),
        };
        title.set_title(&format!("Reply to {}", n.title));
        title.set_subtitle(&format!("{} · {}", n.app_name, device.name));
        message.set_label(if n.body.is_empty() { &n.title } else { &n.body });
        entry.grab_focus();

        let (bridge, window, button) = (bridge.clone(), window.clone(), send.clone());
        let reply = Rc::new(move || {
            let text = entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            send.set_sensitive(false);
            entry.set_sensitive(false);
            let (f, k) = (from.clone(), key.clone());
            let call = bridge.call(&client, move |c| async move { c.reply_notification(f, k, text).await });
            let (window, send, entry, toasts) = (window.clone(), send.clone(), entry.clone(), toasts.clone());
            glib::spawn_future_local(async move {
                match call.await {
                    Ok(()) => window.close(),
                    Err(e) => {
                        toasts.add_toast(adw::Toast::new(&glib::markup_escape_text(&crate::fmt::capitalize(&e))));
                        send.set_sensitive(true);
                        entry.set_sensitive(true);
                    }
                }
            });
        });
        button.connect_clicked(move |_| reply());
    });
}

/// The notification can't be answered any more (or the service is gone).
fn gone(toasts: &adw::ToastOverlay, entry: &gtk::Entry, why: &str) {
    entry.set_sensitive(false);
    toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(why)).timeout(0).build());
}
