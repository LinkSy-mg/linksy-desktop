//! Connect by USB Cable: the steps from "plug it in" to "connected", each showing whether
//! it's done, under way or waiting, live, with what to do next (docs/protocol.md §4.11). The
//! service does the work (`usb_status`, `usb_connect`); this only follows along.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::{UsbState, UsbStatus};

use super::Window;

/// One step: its number (waiting), a spinner (under way) or a check (done).
struct Step {
    row: adw::ActionRow,
    mark: gtk::Stack,
}

impl Step {
    fn new(n: u32, title: &str) -> Self {
        let mark = gtk::Stack::builder().valign(gtk::Align::Center).width_request(24).build();
        mark.add_named(&gtk::Label::builder().label(n.to_string()).css_classes(["step-number"]).build(), Some("todo"));
        mark.add_named(&adw::Spinner::new(), Some("busy"));
        mark.add_named(&gtk::Image::builder().icon_name("object-select-symbolic").css_classes(["success"]).build(), Some("done"));
        let row = adw::ActionRow::builder().title(title).subtitle_lines(4).build();
        row.add_prefix(&mark);
        Step { row, mark }
    }

    fn set(&self, state: &str, subtitle: &str) {
        self.mark.set_visible_child_name(state);
        self.row.set_subtitle(&glib::markup_escape_text(subtitle));
        if state == "todo" {
            self.row.add_css_class("dim-step");
        } else {
            self.row.remove_css_class("dim-step");
        }
    }
}

impl Window {
    pub(super) fn show_usb_cable(self: &Rc<Self>) {
        let dialog = adw::Dialog::builder().title("Connect by USB Cable").content_width(500).build();
        let intro = gtk::Label::builder()
            .label("A faster webcam and microphone, and no Wi-Fi needed. Nothing to turn on in the phone's settings.")
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let access = Step::new(1, "Let Linksy Use the Cable");
        let allow = gtk::Button::builder().label("Allow").valign(gtk::Align::Center).css_classes(["suggested-action"]).visible(false).build();
        access.row.add_suffix(&allow);
        let plug = Step::new(2, "Plug In Your Phone");
        let switch = Step::new(3, "Switch It to Linksy");
        let connect = gtk::Button::builder().label("Connect").valign(gtk::Align::Center).css_classes(["suggested-action"]).visible(false).build();
        switch.row.add_suffix(&connect);
        let open = Step::new(4, "Open Linksy on the Phone");
        let done = Step::new(5, "Connected");
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        for step in [&access, &plug, &switch, &open, &done] {
            list.append(&step.row);
        }
        let unpaired = adw::Banner::builder().title("Pair your phone over Wi-Fi first; the cable then connects it.").revealed(false).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(6).margin_bottom(24).margin_start(24).margin_end(24).build();
        body.append(&intro);
        body.append(&list);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.add_top_bar(&unpaired);
        view.set_content(Some(&body));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        dialog.set_child(Some(&toasts));

        // The phone Connect acts on: the one the last update showed.
        let phone_key: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        let (w, key, t) = (self.clone(), phone_key.clone(), toasts.clone());
        connect.connect_clicked(move |b| {
            let (Some(c), Some(k)) = (w.client(), key.borrow().clone()) else { return };
            b.set_sensitive(false);
            let call = w.bridge.call(&c, move |c| async move { c.usb_connect(&k).await });
            let (b, t) = (b.clone(), t.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = call.await {
                    t.add_toast(adw::Toast::new(&glib::markup_escape_text(&e)));
                }
                b.set_sensitive(true);
            });
        });
        let t = toasts.clone();
        allow.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_child(Some(&adw::Spinner::new()));
            let (b, t) = (b.clone(), t.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = super::setup::allow_usb().await {
                    t.add_toast(adw::Toast::builder().title(glib::markup_escape_text(&e)).timeout(8).build());
                }
                b.set_label("Allow");
                b.set_sensitive(true);
            });
        });

        // Follow along every second while the dialog is open.
        // Eight things in a tuple. It started as two.
        let steps = Rc::new((access, plug, switch, open, done, allow, connect, unpaired));
        let w = Rc::downgrade(self);
        let refresh = {
            let (steps, key) = (steps.clone(), phone_key.clone());
            move |status: Option<UsbStatus>| {
                let Some(w) = w.upgrade() else { return };
                let (access, plug, switch, open, done, allow, connect, unpaired) = &*steps;
                let devices = w.devices();
                let cabled = devices.iter().find(|d| d.trusted && d.connected && d.usb);
                unpaired.set_revealed(!devices.iter().any(|d| d.trusted && d.device.os == "android"));
                let ready = super::setup::usb_ready();
                allow.set_visible(ready.is_err() && cabled.is_none());
                match &ready {
                    _ if cabled.is_some() => access.set("done", "Allowed"),
                    Ok(()) => access.set("done", "Allowed"),
                    Err(why) => access.set("todo", why),
                }
                let status = status.unwrap_or_default();
                // The phone furthest along.
                let phone = status.phones.iter().max_by_key(|p| match p.state {
                    UsbState::Accessory => 3,
                    UsbState::Switching => 2,
                    UsbState::Plugged | UsbState::Failed => 1,
                    UsbState::Declined => 0,
                });
                *key.borrow_mut() = phone.filter(|p| p.state != UsbState::Accessory).map(|p| p.key.clone());
                if !status.watching && cabled.is_none() {
                    plug.set("todo", "Linksy's service isn't watching USB. Restart Linksy, then try again.");
                } else {
                    match (cabled, phone) {
                        (Some(d), _) => plug.set("done", &format!("{} is plugged in", d.device.name)),
                        (None, Some(p)) => plug.set("done", &format!("{} is plugged in", p.name)),
                        (None, None) => plug.set("busy", "Unlock the phone and plug it in. Use a cable that carries data: some only charge."),
                    }
                }
                connect.set_visible(false);
                match (cabled, phone.map(|p| (p.state, p.error.clone()))) {
                    (Some(_), _) | (None, Some((UsbState::Accessory, _))) => switch.set("done", "Switched: file transfer by cable is off until you unplug it"),
                    (None, Some((UsbState::Switching, _))) => switch.set("busy", "Switching…"),
                    (None, Some((UsbState::Failed, error))) => {
                        connect.set_label("Try Again");
                        connect.set_visible(ready.is_ok());
                        switch.set("todo", &format!("Didn't work: {}. Unplug the phone, plug it in again, and try again.", error.unwrap_or_default()));
                    }
                    (None, Some((state, _))) => {
                        connect.set_label("Connect");
                        connect.set_visible(ready.is_ok());
                        let why = if state == UsbState::Declined { "You chose Not This Phone before. " } else { "" };
                        switch.set("todo", &format!("{why}File transfer by cable stops until you unplug it."));
                    }
                    (None, None) => switch.set("todo", "File transfer by cable stops until you unplug it."),
                }
                match (cabled, phone.map(|p| p.state)) {
                    (Some(_), _) => open.set("done", "Opened"),
                    (None, Some(UsbState::Accessory)) => open.set("busy", "The phone asks which app to open: choose Linksy, then Always. Nothing asked? Unlock the phone, or unplug it and plug it in again."),
                    _ => open.set("todo", "The phone asks which app to open: choose Linksy, then Always."),
                }
                match cabled {
                    Some(d) => done.set("done", &format!("{} is connected by USB. Start the webcam or microphone from its page.", d.device.name)),
                    None => done.set("todo", "Your phone's page then says Connected by USB."),
                }
            }
        };
        let refresh = Rc::new(refresh);
        let poll = {
            let (w, refresh) = (Rc::downgrade(self), refresh.clone());
            move || {
                let Some(w) = w.upgrade() else { return };
                let Some(c) = w.client() else { return refresh(None) };
                let call = w.bridge.call(&c, |c| async move { c.usb_status().await });
                let refresh = refresh.clone();
                glib::spawn_future_local(async move { refresh(call.await.ok()) });
            }
        };
        poll();
        let timer = glib::timeout_add_local(Duration::from_secs(1), move || {
            poll();
            glib::ControlFlow::Continue
        });
        let timer = std::cell::Cell::new(Some(timer));
        dialog.connect_closed(move |_| {
            if let Some(t) = timer.take() {
                t.remove();
            }
        });
        dialog.present(Some(&self.window));
    }
}
