//! Connect by USB Cable: the steps from "plug it in" to "connected", each showing whether
//! it's done, under way or waiting, live, with what to do next (docs/protocol.md §4.11). The
//! service does the work (`usb_status`, `usb_connect`); this only follows along.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;
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
        let dialog = adw::Dialog::builder().title(t!("Connect by USB Cable")).content_width(500).build();
        let intro = gtk::Label::builder()
            .label(t!("A faster webcam and microphone, and no Wi-Fi needed. Nothing to turn on in the phone's settings."))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let access = Step::new(1, t!("Let Linksy Use the Cable"));
        let allow = gtk::Button::builder().label(t!("Allow")).valign(gtk::Align::Center).css_classes(["suggested-action"]).visible(false).build();
        access.row.add_suffix(&allow);
        let plug = Step::new(2, t!("Plug In Your Phone"));
        let switch = Step::new(3, t!("Switch It to Linksy"));
        let connect = gtk::Button::builder().label(t!("Connect")).valign(gtk::Align::Center).css_classes(["suggested-action"]).visible(false).build();
        switch.row.add_suffix(&connect);
        let open = Step::new(4, t!("Open Linksy on the Phone"));
        let done = Step::new(5, t!("Connected"));
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        for step in [&access, &plug, &switch, &open, &done] {
            list.append(&step.row);
        }
        // Plugging a phone in asks nothing: this makes one switch on its own
        let auto = adw::SwitchRow::builder().title(t!("Use the Cable Automatically")).visible(false).build();
        let auto_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        auto_list.append(&auto);
        auto.bind_property("visible", &auto_list, "visible").sync_create().build();
        let unpaired = adw::Banner::builder().title(t!("Pair your phone over Wi-Fi first; the cable then connects it.")).revealed(false).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(6).margin_bottom(24).margin_start(24).margin_end(24).build();
        body.append(&intro);
        body.append(&list);
        body.append(&auto_list);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.add_top_bar(&unpaired);
        view.set_content(Some(&body));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        dialog.set_child(Some(&toasts));

        // The phone Connect acts on: the one the last update showed.
        let phone_key: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        // The plugged-in phone the automatic switch is about, and whether a refresh is
        // setting it (not the user)
        let auto_key: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        let auto_showing = Rc::new(std::cell::Cell::new(false));
        let (w, key, showing) = (self.clone(), auto_key.clone(), auto_showing.clone());
        auto.connect_active_notify(move |row| {
            if showing.get() {
                return;
            }
            let (Some(c), Some(k)) = (w.client(), key.borrow().clone()) else { return };
            let on = row.is_active();
            let call = w.bridge.call(&c, move |c| async move { c.usb_auto(&k, on).await });
            glib::spawn_future_local(async move {
                let _ = call.await;
            });
        });
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
                b.set_label(t!("Allow"));
                b.set_sensitive(true);
            });
        });

        // Follow along every second while the dialog is open.
        // Eight things in a tuple. It started as two.
        let steps = Rc::new((access, plug, switch, open, done, allow, connect, unpaired));
        let w = Rc::downgrade(self);
        let refresh = {
            let (steps, key, auto, auto_key, auto_showing) = (steps.clone(), phone_key.clone(), auto.clone(), auto_key.clone(), auto_showing.clone());
            move |status: Option<UsbStatus>| {
                let Some(w) = w.upgrade() else { return };
                let (access, plug, switch, open, done, allow, connect, unpaired) = &*steps;
                let devices = w.devices();
                let cabled = devices.iter().find(|d| d.trusted && d.connected && d.usb);
                unpaired.set_revealed(!devices.iter().any(|d| d.trusted && d.device.os == "android"));
                let ready = super::setup::usb_ready();
                allow.set_visible(ready.is_err() && cabled.is_none());
                match &ready {
                    _ if cabled.is_some() => access.set("done", t!("Allowed")),
                    Ok(()) => access.set("done", t!("Allowed")),
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
                // The automatic switch, for the phone plugged in now
                match phone.filter(|p| p.state != UsbState::Accessory) {
                    Some(p) => {
                        auto_showing.set(true);
                        auto.set_subtitle(&t!("{name} switches to Linksy as soon as it's plugged in, without asking. Off: only when you press Connect.", name = p.name));
                        auto.set_active(p.auto);
                        auto.set_visible(true);
                        auto_showing.set(false);
                        *auto_key.borrow_mut() = Some(p.key.clone());
                    }
                    None => auto.set_visible(cabled.is_some() && auto.is_visible()),
                }
                if !status.watching && cabled.is_none() {
                    plug.set("todo", t!("Linksy's service isn't watching USB. Restart Linksy, then try again."));
                } else {
                    match (cabled, phone) {
                        (Some(d), _) => plug.set("done", &t!("{name} is plugged in", name = d.device.name)),
                        (None, Some(p)) => plug.set("done", &t!("{name} is plugged in", name = p.name)),
                        (None, None) => plug.set("busy", t!("Unlock the phone and plug it in. Use a cable that carries data: some only charge.")),
                    }
                }
                connect.set_visible(false);
                match (cabled, phone.map(|p| (p.state, p.error.clone()))) {
                    (Some(_), _) | (None, Some((UsbState::Accessory, _))) => switch.set("done", t!("Switched: file transfer by cable is off until you unplug it")),
                    (None, Some((UsbState::Switching, _))) => switch.set("busy", t!("Switching…")),
                    (None, Some((UsbState::Failed, error))) => {
                        connect.set_label(t!("Try Again"));
                        connect.set_visible(ready.is_ok());
                        switch.set("todo", &t!("Didn't work: {error}. Unplug the phone, plug it in again, and try again.", error = error.unwrap_or_default()));
                    }
                    (None, Some((state, _))) => {
                        connect.set_label(t!("Connect"));
                        connect.set_visible(ready.is_ok());
                        let why = if state == UsbState::Declined { t!("You chose Not This Phone before. File transfer by cable stops until you unplug it.") } else { t!("File transfer by cable stops until you unplug it.") };
                        switch.set("todo", why);
                    }
                    (None, None) => switch.set("todo", t!("File transfer by cable stops until you unplug it.")),
                }
                match (cabled, phone.map(|p| p.state)) {
                    (Some(_), _) => open.set("done", t!("Opened")),
                    (None, Some(UsbState::Accessory)) => open.set("busy", t!("The phone asks which app to open: choose Linksy, then Always. Nothing asked? Unlock the phone, or unplug it and plug it in again.")),
                    _ => open.set("todo", t!("The phone asks which app to open: choose Linksy, then Always.")),
                }
                match cabled {
                    Some(d) => done.set("done", &t!("{name} is connected by USB. Start the webcam or microphone from its page.", name = d.device.name)),
                    None => done.set("todo", t!("Your phone's page then says Connected by USB.")),
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
