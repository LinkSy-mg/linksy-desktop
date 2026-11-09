//! A phone's controls on its page (docs/protocol.md §4.24): its three volumes, the ringer, Do
//! Not Disturb and the flashlight, and keeping Do Not Disturb in step with this computer's.
//! The service watches the phone; this shows what it last said and asks for what the user changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use linksy_i18n::t;
use linksy_ipc::PhoneControlsInfo;
use linksy_ipc::linksy_core::{ControlsChange, Dnd, Fingerprint, Ringer};

use super::Window;

/// A slider moved by the user is left alone this long, so the phone's older answers don't pull it back.
const HELD: Duration = Duration::from_millis(1200);
/// A slider being dragged asks the phone this often, no more.
const EVERY: Duration = Duration::from_millis(150);

const RINGERS: [Ringer; 3] = [Ringer::Normal, Ringer::Vibrate, Ringer::Silent];
const DNDS: [Dnd; 4] = [Dnd::Off, Dnd::Priority, Dnd::Alarms, Dnd::Silence];

#[derive(Clone)]
pub struct ControlsPanel {
    pub root: gtk::Box,
    media: Level,
    ringtone: Level,
    alarm: Level,
    ringer: adw::ComboRow,
    dnd: adw::ComboRow,
    flashlight: adw::SwitchRow,
    sync: adw::SwitchRow,
    /// Set while the rows are being filled from what the phone said, so that isn't sent back.
    quiet: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct Level {
    row: adw::ActionRow,
    scale: gtk::Scale,
    touched: Rc<Cell<Option<Instant>>>,
}

impl ControlsPanel {
    /// `change` is called with what the user changed, `sync` with the new setting of keeping Do
    /// Not Disturb in step.
    pub fn new(change: Rc<dyn Fn(ControlsChange)>, sync: Rc<dyn Fn(bool)>) -> Self {
        let quiet: Rc<Cell<bool>> = Rc::default();
        // Many moves of a slider make a few asks
        let pending: Rc<RefCell<ControlsChange>> = Rc::default();
        let timer: Rc<Cell<bool>> = Rc::default();
        let ask: Rc<dyn Fn(ControlsChange)> = {
            let (change, pending, timer) = (change.clone(), pending.clone(), timer.clone());
            Rc::new(move |c: ControlsChange| {
                let mut p = pending.borrow_mut();
                *p = ControlsChange {
                    media: c.media.or(p.media),
                    ringtone: c.ringtone.or(p.ringtone),
                    alarm: c.alarm.or(p.alarm),
                    ringer: c.ringer.or(p.ringer),
                    dnd: c.dnd.or(p.dnd),
                    flashlight: c.flashlight.or(p.flashlight),
                };
                if timer.replace(true) {
                    return;
                }
                let (change, pending, timer) = (change.clone(), pending.clone(), timer.clone());
                glib::timeout_add_local_once(EVERY, move || {
                    timer.set(false);
                    let c = std::mem::take(&mut *pending.borrow_mut());
                    change(c);
                });
            })
        };

        let level = |icon: &str, title: &str, set: fn(&mut ControlsChange, f64)| -> Level {
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
            // The wheel and Page Up/Down move by the page increment, 10 steps unless set
            scale.adjustment().set_page_increment(0.05);
            scale.set_draw_value(false);
            scale.set_width_request(220);
            scale.set_valign(gtk::Align::Center);
            scale.update_property(&[gtk::accessible::Property::Label(title)]);
            let row = adw::ActionRow::builder().title(title).build();
            row.add_prefix(&gtk::Image::builder().icon_name(icon).build());
            row.add_suffix(&scale);
            let touched: Rc<Cell<Option<Instant>>> = Rc::default();
            let (ask, quiet, touched2) = (ask.clone(), quiet.clone(), touched.clone());
            scale.connect_value_changed(move |s| {
                if quiet.get() {
                    return;
                }
                touched2.set(Some(Instant::now()));
                let mut c = ControlsChange::default();
                set(&mut c, s.value());
                ask(c);
            });
            Level { row, scale, touched }
        };
        let media = level("audio-speakers-symbolic", t!("Media Volume"), |c, v| c.media = Some(v));
        let ringtone = level("phone-symbolic", t!("Ringtone Volume"), |c, v| c.ringtone = Some(v));
        let alarm = level("alarm-symbolic", t!("Alarm Volume"), |c, v| c.alarm = Some(v));

        let ringer = adw::ComboRow::builder()
            .title(t!("Ringer"))
            .model(&gtk::StringList::new(&[t!("Normal"), t!("Vibrate"), t!("Silent")]))
            .build();
        ringer.add_prefix(&gtk::Image::builder().icon_name("preferences-system-notifications-symbolic").build());
        let (a, q) = (ask.clone(), quiet.clone());
        ringer.connect_selected_notify(move |row| {
            if q.get() {
                return;
            }
            if let Some(r) = RINGERS.get(row.selected() as usize) {
                a(ControlsChange { ringer: Some(*r), ..ControlsChange::default() });
            }
        });
        let dnd = adw::ComboRow::builder()
            .title(t!("Do Not Disturb"))
            .subtitle_lines(2)
            .model(&gtk::StringList::new(&[t!("Off"), t!("Priority Only"), t!("Alarms Only"), t!("Total Silence")]))
            .build();
        dnd.add_prefix(&gtk::Image::builder().icon_name("notifications-disabled-symbolic").build());
        let (a, q) = (ask.clone(), quiet.clone());
        dnd.connect_selected_notify(move |row| {
            if q.get() {
                return;
            }
            if let Some(d) = DNDS.get(row.selected() as usize) {
                a(ControlsChange { dnd: Some(*d), ..ControlsChange::default() });
            }
        });
        let flashlight = adw::SwitchRow::builder().title(t!("Flashlight")).build();
        flashlight.add_prefix(&gtk::Image::builder().icon_name("flashlight-symbolic").build());
        let (a, q) = (ask.clone(), quiet.clone());
        flashlight.connect_active_notify(move |row| {
            if !q.get() {
                a(ControlsChange { flashlight: Some(row.is_active()), ..ControlsChange::default() });
            }
        });
        let sync_row = adw::SwitchRow::builder().title(t!("Keep Do Not Disturb in Sync")).subtitle(t!("Turning it on or off here or on the phone does the same on the other")).subtitle_lines(3).build();
        sync_row.add_prefix(&gtk::Image::builder().icon_name("emblem-synchronizing-symbolic").build());
        let q = quiet.clone();
        sync_row.connect_active_notify(move |row| {
            if !q.get() {
                sync(row.is_active());
            }
        });

        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        for row in [media.row.upcast_ref::<gtk::Widget>(), ringtone.row.upcast_ref(), alarm.row.upcast_ref(), ringer.upcast_ref(), dnd.upcast_ref(), flashlight.upcast_ref(), sync_row.upcast_ref()] {
            list.append(row);
        }
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).visible(false).build();
        root.append(&gtk::Label::builder().label(t!("Sound and Silence")).xalign(0.0).css_classes(["title-4"]).build());
        root.append(&list);
        Self { root, media, ringtone, alarm, ringer, dnd, flashlight, sync: sync_row, quiet }
    }

    /// What the phone last said; nothing hides the panel.
    pub fn show(&self, info: Option<&PhoneControlsInfo>) {
        let Some(info) = info else {
            self.root.set_visible(false);
            return;
        };
        let Some(s) = info.state else {
            self.root.set_visible(false);
            return;
        };
        self.quiet.set(true);
        for (level, value) in [(&self.media, s.media), (&self.ringtone, s.ringtone), (&self.alarm, s.alarm)] {
            let held = level.touched.get().is_some_and(|t| t.elapsed() < HELD);
            if !held {
                level.scale.set_value(value);
            }
        }
        self.ringer.set_selected(RINGERS.iter().position(|r| *r == s.ringer).map_or(gtk::INVALID_LIST_POSITION, |p| p as u32));
        self.dnd.set_selected(DNDS.iter().position(|d| *d == s.dnd).map_or(gtk::INVALID_LIST_POSITION, |p| p as u32));
        // Android lets Linksy change it only once the user has said so, on the phone
        self.dnd.set_sensitive(s.dnd_access);
        self.dnd.set_subtitle(if s.dnd_access { "" } else { t!("Turn on Do Not Disturb access for Linksy in the phone's settings to change it from here") });
        self.flashlight.set_visible(s.flashlight.is_some());
        self.flashlight.set_active(s.flashlight == Some(true));
        self.sync.set_visible(info.can_sync);
        self.sync.set_sensitive(s.dnd_access);
        self.sync.set_active(info.sync_dnd);
        self.quiet.set(false);
        self.root.set_visible(true);
    }
}

impl Window {
    /// Ask the service how the phone's controls are, for its page.
    pub(super) fn load_controls(self: &Rc<Self>, id: Fingerprint) {
        if !self.device(&id).is_some_and(|d| d.trusted && super::can(&d.device, "phone_controls")) {
            return;
        }
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.phone_controls(id.to_string()).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(info) = call.await {
                w.state.borrow_mut().controls.insert(id, info);
                w.show_controls();
            }
        });
    }

    /// Show the selected phone's controls as last known.
    pub(super) fn show_controls(&self) {
        let info = self.state.borrow().selected.and_then(|id| self.state.borrow().controls.get(&id).cloned());
        self.page.show_controls(info.as_ref());
    }

    /// The phone said how its controls are.
    pub(super) fn controls_said(self: &Rc<Self>, id: Fingerprint, state: linksy_ipc::linksy_core::ControlsState) {
        let known = {
            let mut s = self.state.borrow_mut();
            match s.controls.get_mut(&id) {
                Some(info) => {
                    info.state = Some(state);
                    true
                }
                None => false,
            }
        };
        // A phone not asked about yet: ask, for it also says whether Do Not Disturb follows
        if known {
            self.show_controls();
        } else {
            self.load_controls(id);
        }
    }

    pub(super) fn change_controls(self: &Rc<Self>, id: Fingerprint, change: ControlsChange) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.set_phone_controls(id.to_string(), change).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(id));
                w.load_controls(id);
            }
        });
    }

    pub(super) fn keep_dnd_in_sync(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.set_dnd_sync(id.to_string(), on).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => {
                    if let Some(info) = w.state.borrow_mut().controls.get_mut(&id) {
                        info.sync_dnd = on;
                    }
                }
                Err(e) => {
                    w.toast_error(&e, Some(id));
                    w.load_controls(id);
                }
            }
        });
    }
}

impl Window {
    /// Ask the service whether this computer's notifications go to the phone, for its page.
    pub(super) fn load_forward(self: &Rc<Self>, id: Fingerprint) {
        if !self.device(&id).is_some_and(|d| d.trusted && super::can(&d.device, "computer_notifications")) {
            return;
        }
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.computer_notifications(id.to_string()).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(info) = call.await {
                w.state.borrow_mut().forward.insert(id, info);
                w.show_forward();
            }
        });
    }

    pub(super) fn show_forward(&self) {
        let Some(d) = self.selected_device() else { return };
        let info = self.state.borrow().forward.get(&d.device.id).cloned();
        self.page.show_forward(&d.device.name, info.as_ref().filter(|_| super::can(&d.device, "computer_notifications")));
    }

    pub(super) fn forward_notifications(self: &Rc<Self>, id: Fingerprint, on: bool) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.set_computer_notifications(id.to_string(), on).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => {
                    if let Some(info) = w.state.borrow_mut().forward.get_mut(&id) {
                        info.on = on;
                    }
                }
                Err(e) => {
                    w.toast_error(&e, Some(id));
                    w.load_forward(id);
                }
            }
        });
    }
}

/// What a phone refused to change, in a few words.
pub(super) fn controls_what(what: &str) -> String {
    match what {
        "media" => t!("the media volume"),
        "ringtone" => t!("the ringtone volume"),
        "alarm" => t!("the alarm volume"),
        "ringer" => t!("the ringer"),
        "dnd" => t!("Do Not Disturb"),
        "flashlight" => t!("the flashlight"),
        other => other,
    }
    .to_string()
}
