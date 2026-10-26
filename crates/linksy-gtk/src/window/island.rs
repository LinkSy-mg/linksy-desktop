//! The sidebar's island, as on the phone: Transfers, Send Files and Add Device in one pill.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::linksy_core::{DeviceView, Fingerprint};
use linksy_i18n::t;

use super::kind_icon;

#[derive(Clone)]
pub(super) enum Pick {
    Transfers,
    Send(Fingerprint),
    AddDevice,
    PairAddress,
    UsbCable,
}

#[derive(Clone, Copy, PartialEq)]
enum Card {
    Send,
    Add,
}

type Handler = RefCell<Option<Rc<dyn Fn(Pick)>>>;

pub(super) struct Island {
    pub root: gtk::Box,
    card: gtk::Revealer,
    card_box: gtk::Box,
    transfers: gtk::Button,
    ring: gtk::DrawingArea,
    progress: Rc<Cell<Option<f64>>>,
    icon: gtk::Image,
    count: gtk::Label,
    middle: gtk::Button,
    middle_content: adw::ButtonContent,
    right: gtk::Button,
    open: Cell<Option<Card>>,
    devices: RefCell<Vec<DeviceView>>,
    handler: Handler,
}

impl Island {
    pub fn new() -> Rc<Self> {
        let progress: Rc<Cell<Option<f64>>> = Rc::default();
        let ring = gtk::DrawingArea::builder().content_width(40).content_height(40).css_classes(["island-ring"]).build();
        let p = progress.clone();
        ring.set_draw_func(move |area, cr, w, h| {
            let Some(v) = p.get() else { return };
            let c = area.color();
            let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0 - 2.0);
            cr.set_line_width(3.0);
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.18);
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 1.0);
            let start = -std::f64::consts::FRAC_PI_2;
            cr.arc(cx, cy, r, start, start + v.clamp(0.0, 1.0) * std::f64::consts::TAU);
            let _ = cr.stroke();
        });
        let icon = gtk::Image::builder().icon_name("network-transmit-receive-symbolic").build();
        let count = gtk::Label::builder().css_classes(["island-count", "numeric"]).visible(false).build();
        let center = gtk::Box::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        center.append(&icon);
        center.append(&count);
        let bubble = gtk::Overlay::builder().child(&ring).build();
        bubble.add_overlay(&center);
        let transfers = gtk::Button::builder().child(&bubble).tooltip_text(t!("Transfers")).css_classes(["flat", "island-part", "island-transfers"]).build();

        let middle_content = adw::ButtonContent::builder().icon_name("document-send-symbolic").label(t!("Send Files")).build();
        let middle = gtk::Button::builder().child(&middle_content).hexpand(true).css_classes(["island-main"]).build();
        let right = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text(t!("Add Device")).css_classes(["flat", "island-part"]).build();
        let pill = gtk::Box::builder().spacing(4).build();
        pill.append(&transfers);
        pill.append(&middle);
        pill.append(&right);

        let card_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(14).margin_start(10).margin_end(10).margin_bottom(8).build();
        let card = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).transition_duration(300).child(&card_box).build();
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["island"]).build();
        root.append(&card);
        root.append(&pill);

        let island = Rc::new(Self {
            root,
            card,
            card_box,
            transfers,
            ring,
            progress,
            icon,
            count,
            middle,
            middle_content,
            right,
            open: Cell::new(None),
            devices: RefCell::default(),
            handler: RefCell::default(),
        });
        let i = Rc::downgrade(&island);
        island.transfers.connect_clicked(move |_| {
            if let Some(i) = i.upgrade() {
                i.close();
                i.pick(Pick::Transfers);
            }
        });
        for (button, middle) in [(&island.middle, true), (&island.right, false)] {
            let i = Rc::downgrade(&island);
            button.connect_clicked(move |_| {
                let Some(i) = i.upgrade() else { return };
                // Before the first device, Add takes the middle
                let none = i.devices.borrow().is_empty();
                i.toggle(if middle != none { Card::Send } else { Card::Add });
            });
        }
        island
    }

    pub fn set_handler(&self, f: impl Fn(Pick) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(f));
    }

    fn pick(&self, pick: Pick) {
        let f = self.handler.borrow().clone();
        if let Some(f) = f {
            f(pick);
        }
    }

    pub fn set_devices(self: &Rc<Self>, paired: &[DeviceView]) {
        *self.devices.borrow_mut() = paired.to_vec();
        let none = paired.is_empty();
        self.middle_content.set_label(if none { t!("Add Device") } else { t!("Send Files") });
        self.middle_content.set_icon_name(if none { "list-add-symbolic" } else { "document-send-symbolic" });
        self.right.set_icon_name(if none { "document-send-symbolic" } else { "list-add-symbolic" });
        self.right.set_tooltip_text(Some(if none { t!("Send Files") } else { t!("Add Device") }));
        if self.open.get() == Some(Card::Send) {
            self.fill(Card::Send);
        }
    }

    pub fn set_moving(&self, files: usize, progress: Option<f64>) {
        let moving = files > 0;
        self.icon.set_visible(!moving);
        self.count.set_visible(moving);
        self.count.set_label(&files.to_string());
        self.progress.set(moving.then(|| progress.unwrap_or(0.0)));
        self.ring.queue_draw();
    }

    pub fn set_selected(&self, on: bool) {
        if on {
            self.transfers.add_css_class("selected");
        } else {
            self.transfers.remove_css_class("selected");
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get().is_some()
    }

    pub fn close(&self) {
        self.open.set(None);
        self.card.set_reveal_child(false);
        self.root.remove_css_class("open");
    }

    fn toggle(self: &Rc<Self>, card: Card) {
        if self.open.get() == Some(card) {
            self.close();
            return;
        }
        self.fill(card);
        self.open.set(Some(card));
        self.root.add_css_class("open");
        self.card.set_reveal_child(true);
    }

    fn fill(self: &Rc<Self>, card: Card) {
        while let Some(child) = self.card_box.first_child() {
            self.card_box.remove(&child);
        }
        let title = gtk::Label::builder().xalign(0.0).margin_bottom(8).margin_start(4).css_classes(["title-4"]).build();
        self.card_box.append(&title);
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["island-list"]).build();
        self.card_box.append(&list);
        let mut picks = Vec::new();
        let devices = self.devices.borrow().clone();
        if card == Card::Send && !devices.is_empty() {
            title.set_label(t!("Send Files"));
            for d in &devices {
                list.append(&option(kind_icon(d.device.kind), &d.device.name, Some("document-send-symbolic")));
                picks.push(Pick::Send(d.device.id));
            }
        } else {
            title.set_label(t!("Add Device"));
            for (icon, label, pick) in [
                ("list-add-symbolic", t!("Add Device…"), Pick::AddDevice),
                ("network-server-symbolic", t!("Pair by Address…"), Pick::PairAddress),
                ("drive-harddisk-usb-symbolic", t!("Connect by USB Cable…"), Pick::UsbCable),
            ] {
                list.append(&option(icon, label, None));
                picks.push(pick);
            }
        }
        let i = Rc::downgrade(self);
        list.connect_row_activated(move |_, row| {
            let Some(i) = i.upgrade() else { return };
            let Some(pick) = picks.get(row.index() as usize).cloned() else { return };
            i.close();
            glib::timeout_add_local_once(std::time::Duration::from_millis(220), move || i.pick(pick));
        });
    }
}

fn option(icon: &str, label: &str, end: Option<&str>) -> gtk::ListBoxRow {
    let line = gtk::Box::builder().spacing(12).build();
    line.append(&gtk::Image::builder().icon_name(icon).css_classes(["island-option-icon"]).build());
    line.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build());
    if let Some(end) = end {
        line.append(&gtk::Image::builder().icon_name(end).css_classes(["island-accent"]).build());
    }
    gtk::ListBoxRow::builder().child(&line).build()
}
