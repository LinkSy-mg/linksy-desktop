//! Transfers, a page of its own as on the phone: speed, files on their way, and recent ones.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gio, glib};
use linksy_ipc::linksy_core::{DeviceKind, Fingerprint, Uuid};
use linksy_ipc::{HistoryEntry, HistoryStatus, Shared};
use linksy_i18n::t;

use super::page::{Action, day_label, time_label};
use super::{Progress, kind_icon, speed};
use crate::{fmt, mark};

#[derive(Clone)]
pub(super) struct Moving {
    pub id: Uuid,
    pub peer: Fingerprint,
    pub peer_name: String,
    pub incoming: bool,
    pub import: bool,
    pub files: Vec<(String, Option<u64>)>,
    pub total: u64,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Route {
    Wifi,
    Away,
    Usb,
    DirectSetup,
    Direct,
}

/// Most files listed for one transfer at a time, around the one on its way.
const LISTED: usize = 30;
const RECENT: usize = 30;

type Handler = RefCell<Option<Rc<dyn Fn(Action)>>>;

struct Live {
    number: gtk::Label,
    unit: gtk::Label,
    waiting: gtk::Label,
    percent: gtk::Label,
    flow: Flow,
    feels: gtk::Label,
    feels_row: gtk::Box,
    groups: HashMap<Uuid, Group>,
}

struct Group {
    caps: gtk::Label,
    first: usize,
    rows: Vec<FileRow>,
}

struct FileRow {
    ring: Ring,
    name: gtk::Label,
    detail: gtk::Label,
    check: gtk::Image,
}

pub(super) struct TransfersPage {
    pub root: gtk::ScrolledWindow,
    body: gtk::Box,
    live: RefCell<Option<Live>>,
    open: RefCell<HashSet<Uuid>>,
    handler: Handler,
}

impl TransfersPage {
    pub fn new() -> Rc<Self> {
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(12).margin_bottom(24).margin_start(16).margin_end(16).build();
        let clamp = adw::Clamp::builder().maximum_size(640).child(&body).build();
        let root = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
        Rc::new(Self { root, body, live: RefCell::default(), open: RefCell::default(), handler: RefCell::default() })
    }

    pub fn set_handler(&self, f: impl Fn(Action) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(f));
    }

    fn act(&self, action: Action) {
        let f = self.handler.borrow().clone();
        if let Some(f) = f {
            f(action);
        }
    }

    pub fn show(self: &Rc<Self>, moving: &[Moving], other: DeviceKind, route: Route, recent: &[HistoryEntry], progress: &HashMap<Uuid, Progress>) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        *self.live.borrow_mut() = None;
        if moving.is_empty() {
            self.body.append(&nothing());
        } else {
            let first = &moving[0];
            let (card, mut live) = speed_card(first.incoming, other, route);
            self.body.append(&card);
            self.body.append(&live.feels_row);
            for m in moving {
                let (widget, group) = self.group(m, progress.get(&m.id));
                self.body.append(&widget);
                live.groups.insert(m.id, group);
            }
            *self.live.borrow_mut() = Some(live);
        }
        let ended: Vec<&HistoryEntry> = recent.iter().filter(|e| matches!(e.what, Shared::Files { .. }) && !matches!(e.status, HistoryStatus::InProgress)).take(RECENT).collect();
        if !ended.is_empty() {
            let caps = caps(t!("Recent"));
            caps.set_margin_top(28);
            caps.set_margin_bottom(6);
            self.body.append(&caps);
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["transfers-recent"]).build();
            for e in ended {
                list.append(&self.recent_row(e));
            }
            self.body.append(&list);
        }
        self.update(moving, progress);
    }

    /// Updates in place; true when the page needs `show` again.
    pub fn update(&self, moving: &[Moving], progress: &HashMap<Uuid, Progress>) -> bool {
        let live = self.live.borrow();
        let Some(live) = live.as_ref() else { return !moving.is_empty() };
        let (mut bytes, mut total, mut rate) = (0u64, 0u64, 0.0);
        for m in moving {
            match progress.get(&m.id) {
                Some(p) => {
                    bytes += p.bytes;
                    total += p.total.max(m.total);
                    rate += speed(p);
                }
                None => total += m.total,
            }
        }
        let going = rate > 0.0;
        if going {
            let text = t!("{size}/s", size = fmt::size(rate as u64));
            let (number, unit) = text.split_once(char::is_whitespace).unwrap_or((&text, ""));
            live.number.set_label(number);
            live.unit.set_label(unit);
        }
        live.number.set_visible(going);
        live.unit.set_visible(going);
        live.waiting.set_visible(!going);
        let started = moving.iter().any(|m| progress.contains_key(&m.id));
        live.percent.set_visible(started && total > 0);
        if total > 0 {
            live.percent.set_label(&format!("{}%", (bytes as f64 / total as f64 * 100.0).round() as u64));
        }
        live.flow.speed.set(rate);

        let mb = rate / 1e6;
        let mut lines = Vec::new();
        if mb >= 15.0 {
            lines.push(t!("A 1 GB movie every {seconds} s", seconds = (1000.0 / mb).round() as u64));
            if mb / 3.0 >= 2.0 {
                lines.push(t!("About {count} photos a second", count = (mb / 3.0).round() as u64));
            }
            if mb / 5.0 >= 2.0 {
                lines.push(t!("{count} songs a second", count = (mb / 5.0).round() as u64));
            }
        }
        live.feels_row.set_visible(!lines.is_empty());
        if !lines.is_empty() {
            let turn = (glib::monotonic_time() / 4_000_000) as usize;
            live.feels.set_label(&lines[turn % lines.len()]);
        }

        let mut again = false;
        for m in moving {
            let Some(group) = live.groups.get(&m.id) else {
                again = true;
                continue;
            };
            let p = progress.get(&m.id);
            let sizes: Option<Vec<u64>> = m.files.iter().map(|(_, s)| *s).collect();
            let done = match (&sizes, p) {
                (Some(sizes), Some(p)) => done_files(sizes, p.bytes),
                _ => 0,
            };
            let mut parts = vec![if m.incoming { t!("From {name}", name = m.peer_name) } else { t!("To {name}", name = m.peer_name) }];
            parts.push(match sizes {
                Some(_) => t!("{done} of {total} files", done, total = m.files.len()),
                None => fmt::plural(m.files.len(), if m.import { "photo" } else { "item" }),
            });
            group.caps.set_label(&parts.join(" · ").to_uppercase());
            for (k, row) in group.rows.iter().enumerate() {
                let i = group.first + k;
                let size = m.files[i].1;
                let state = if sizes.is_none() || p.is_none() {
                    FileState::Next
                } else if i < done {
                    FileState::Done
                } else if i == done {
                    FileState::Going
                } else {
                    FileState::Next
                };
                let fraction = match (&sizes, p) {
                    (Some(sizes), Some(p)) => file_fraction(sizes, i, p.bytes),
                    _ => None,
                };
                row.ring.set(state == FileState::Going, fraction);
                row.check.set_visible(state == FileState::Done);
                if state == FileState::Going {
                    row.name.add_css_class("going");
                } else {
                    row.name.remove_css_class("going");
                }
                match (state, size, fraction) {
                    (FileState::Going, Some(size), Some(f)) => {
                        let text = t!("{done} of {total}", done = fmt::size((size as f64 * f) as u64), total = fmt::size(size));
                        row.detail.set_markup(&format!("{} · <b>{}%</b>", glib::markup_escape_text(&text), (f * 100.0).round() as u64));
                    }
                    (_, Some(size), _) => row.detail.set_label(&fmt::size(size)),
                    _ => row.detail.set_label(""),
                }
                row.detail.set_visible(!row.detail.label().is_empty());
            }
            if sizes.is_some() && p.is_some() && done < m.files.len() && !(group.first..group.first + group.rows.len()).contains(&done) {
                again = true;
            }
        }
        again
    }

    fn group(self: &Rc<Self>, m: &Moving, p: Option<&Progress>) -> (gtk::Box, Group) {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(22).build();
        let caps = caps("");
        caps.set_hexpand(true);
        caps.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let cancel = gtk::Button::builder().label(t!("Cancel")).css_classes(["flat", "dim-label"]).build();
        let (w, id) = (Rc::downgrade(self), m.id);
        cancel.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.act(Action::Cancel(id));
            }
        });
        let head = gtk::Box::builder().spacing(8).build();
        head.append(&caps);
        head.append(&cancel);
        root.append(&head);
        let sizes: Option<Vec<u64>> = m.files.iter().map(|(_, s)| *s).collect();
        let done = match (&sizes, p) {
            (Some(sizes), Some(p)) => done_files(sizes, p.bytes),
            _ => 0,
        };
        let first = done.saturating_sub(2).min(m.files.len().saturating_sub(LISTED));
        let mut rows = Vec::new();
        for (name, _) in m.files.iter().skip(first).take(LISTED) {
            let (widget, row) = file_row(name);
            root.append(&widget);
            rows.push(row);
        }
        let more = m.files.len() - first - rows.len();
        if more > 0 {
            let label = gtk::Label::builder().label(t!("and {count} more", count = more)).xalign(0.0).margin_start(60).margin_top(4).css_classes(["dim-label", "caption"]).build();
            root.append(&label);
        }
        (root, Group { caps, first, rows })
    }

    fn recent_row(self: &Rc<Self>, e: &HistoryEntry) -> gtk::ListBoxRow {
        let Shared::Files { files, total, saved, import, .. } = &e.what else { unreachable!("only files are listed") };
        let incoming = e.direction == linksy_ipc::linksy_core::Direction::Incoming;
        let group = files.len() > 1;
        let icon = if group {
            gio::ThemedIcon::new(if incoming { "folder-download-symbolic" } else { "document-send-symbolic" }).upcast::<gio::Icon>()
        } else {
            file_icon(&files[0])
        };
        let ring = Ring::new(&icon);
        let title = if *import {
            fmt::plural(files.len(), "photo")
        } else if group {
            fmt::plural(files.len(), "item")
        } else {
            fmt::file_name(&files[0])
        };
        let today = day_label(e.at) == t!("Today");
        let when = if today { time_label(e.at) } else { day_label(e.at) };
        let mut detail = vec![if incoming { t!("From {name}", name = e.peer_name) } else { t!("To {name}", name = e.peer_name) }];
        if *total > 0 {
            detail.push(fmt::size(*total));
        }
        detail.push(when);
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        text.append(&gtk::Label::builder().label(&title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["transfer-name"]).build());
        text.append(&gtk::Label::builder().label(detail.join(" · ")).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label"]).build());
        let line = gtk::Box::builder().spacing(12).build();
        line.append(&ring.root);
        line.append(&text);
        let failed = match &e.status {
            HistoryStatus::Declined => Some(t!("Declined").to_string()),
            HistoryStatus::Cancelled => Some(t!("Cancelled").to_string()),
            HistoryStatus::Failed { .. } => Some(t!("Failed").to_string()),
            _ => None,
        };
        let arrow = gtk::Image::builder().icon_name("pan-down-symbolic").css_classes(["dim-label"]).build();
        if let Some(why) = &failed {
            line.append(&gtk::Label::builder().label(why).css_classes(["caption", "transfer-failed"]).build());
        } else if group {
            line.append(&arrow);
        }
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        column.append(&line);
        let row = gtk::ListBoxRow::builder().child(&column).activatable(false).build();
        if let HistoryStatus::Failed { reason } = &e.status {
            row.set_tooltip_text(Some(reason));
        }
        let saved = saved.clone();
        if group && failed.is_none() {
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).margin_start(52).css_classes(["transfers-recent"]).build();
            for (i, name) in files.iter().enumerate() {
                let line = gtk::Box::builder().spacing(10).build();
                line.append(&gtk::Image::builder().gicon(&file_icon(name)).css_classes(["dim-label"]).build());
                line.append(&gtk::Label::builder().label(fmt::file_name(name)).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
                let file = gtk::ListBoxRow::builder().child(&line).activatable(saved.get(i).is_some()).build();
                list.append(&file);
            }
            let w = Rc::downgrade(self);
            let paths = saved.clone();
            list.connect_row_activated(move |_, r| {
                if let (Some(w), Some(path)) = (w.upgrade(), paths.get(r.index() as usize)) {
                    w.act(Action::Open(vec![path.clone()]));
                }
            });
            let open = self.open.borrow().contains(&e.id);
            let revealer = gtk::Revealer::builder().reveal_child(open).child(&list).build();
            column.append(&revealer);
            if open {
                arrow.add_css_class("open");
            }
            row.set_activatable(true);
            let (w, id) = (Rc::downgrade(self), e.id);
            row.connect_activate(move |_| {
                let Some(w) = w.upgrade() else { return };
                let open = !revealer.reveals_child();
                revealer.set_reveal_child(open);
                if open {
                    arrow.add_css_class("open");
                    w.open.borrow_mut().insert(id);
                } else {
                    arrow.remove_css_class("open");
                    w.open.borrow_mut().remove(&id);
                }
            });
        } else if incoming && !saved.is_empty() && failed.is_none() {
            row.set_activatable(true);
            let w = Rc::downgrade(self);
            row.connect_activate(move |_| {
                if let Some(w) = w.upgrade() {
                    w.act(Action::Open(saved.clone()));
                }
            });
        }
        row
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FileState {
    Done,
    Going,
    Next,
}

/// Whole files the bytes cover, in order.
fn done_files(sizes: &[u64], bytes: u64) -> usize {
    let mut sum = 0;
    for (i, size) in sizes.iter().enumerate() {
        sum += size;
        if sum > bytes {
            return i;
        }
    }
    sizes.len()
}

/// How far file `i` is, from the bytes past the files before it.
fn file_fraction(sizes: &[u64], i: usize, bytes: u64) -> Option<f64> {
    let size = *sizes.get(i)?;
    if size == 0 {
        return None;
    }
    let before: u64 = sizes[..i].iter().sum();
    Some((bytes.saturating_sub(before) as f64 / size as f64).clamp(0.0, 1.0))
}

fn file_icon(name: &str) -> gio::Icon {
    let (kind, _) = gio::content_type_guess(Some(name), None::<&[u8]>);
    gio::content_type_get_symbolic_icon(&kind)
}

fn caps(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text.to_uppercase()).xalign(0.0).css_classes(["transfers-caps", "dim-label"]).build()
}

fn nothing() -> gtk::Box {
    let badge = gtk::Image::builder().icon_name("network-transmit-receive-symbolic").pixel_size(24).css_classes(["transfers-badge"]).build();
    let title = gtk::Label::builder().label(t!("Nothing on its way")).xalign(0.0).css_classes(["title-4"]).build();
    let body = gtk::Label::builder().label(t!("Files you send or receive show up here, with their speed.")).xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).valign(gtk::Align::Center).hexpand(true).build();
    text.append(&title);
    text.append(&body);
    let card = gtk::Box::builder().spacing(16).css_classes(["card", "transfers-card"]).build();
    card.append(&badge);
    card.append(&text);
    card
}

fn speed_card(incoming: bool, other: DeviceKind, route: Route) -> (gtk::Box, Live) {
    let number = gtk::Label::builder().valign(gtk::Align::Baseline).css_classes(["transfer-speed", "numeric"]).build();
    let unit = gtk::Label::builder().valign(gtk::Align::Baseline).css_classes(["dim-label"]).build();
    let waiting = gtk::Label::builder()
        .label(if route == Route::DirectSetup { t!("Setting up Wi-Fi Direct") } else { t!("Waiting to start") })
        .xalign(0.0)
        .valign(gtk::Align::Baseline)
        .css_classes(["transfer-waiting"])
        .build();
    let percent = gtk::Label::builder().valign(gtk::Align::Baseline).css_classes(["transfer-percent", "numeric"]).build();
    let top = gtk::Box::builder().spacing(6).build();
    top.append(&number);
    top.append(&unit);
    top.append(&waiting);
    top.append(&gtk::Box::builder().hexpand(true).build());
    top.append(&percent);

    let flow = Flow::new(route == Route::DirectSetup);
    let lane = gtk::Overlay::builder().child(&flow.area).hexpand(true).build();
    lane.add_overlay(&route_pill(route));
    let me = end("computer-symbolic", "me");
    let them = end(kind_icon(other), "other");
    let (from, to) = if incoming { (them, me) } else { (me, them) };
    let road = gtk::Box::builder().spacing(10).margin_top(12).build();
    road.append(&from);
    road.append(&lane);
    road.append(&to);

    let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["card", "transfers-card"]).build();
    card.append(&top);
    card.append(&road);

    let feels = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["transfers-feels"]).build();
    let feels_row = gtk::Box::builder().spacing(8).margin_top(12).margin_start(8).visible(false).build();
    feels_row.append(&gtk::Image::builder().icon_name("thunderbolt-symbolic").css_classes(["transfers-feels"]).build());
    feels_row.append(&feels);
    (card, Live { number, unit, waiting, percent, flow, feels, feels_row, groups: HashMap::new() })
}

fn end(icon: &str, who: &str) -> gtk::Image {
    gtk::Image::builder().icon_name(icon).pixel_size(18).valign(gtk::Align::Center).css_classes(["lane-end", who]).build()
}

fn route_pill(route: Route) -> gtk::Box {
    let (icon, text) = match route {
        Route::Direct => ("thunderbolt-symbolic", t!("Wi-Fi Direct")),
        Route::DirectSetup => ("network-wireless-acquiring-symbolic", t!("Setting up Wi-Fi Direct")),
        Route::Usb => ("drive-harddisk-usb-symbolic", t!("USB cable")),
        Route::Away => ("web-browser-symbolic", t!("Away from home")),
        Route::Wifi => ("network-wireless-symbolic", t!("Same Wi-Fi")),
    };
    let pill = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).valign(gtk::Align::Center).css_classes(["route-pill"]).build();
    if matches!(route, Route::Direct | Route::DirectSetup) {
        pill.add_css_class("direct");
    }
    pill.append(&gtk::Image::builder().icon_name(icon).pixel_size(12).build());
    pill.append(&gtk::Label::new(Some(text)));
    pill
}

fn file_row(name: &str) -> (gtk::Box, FileRow) {
    let ring = Ring::new(&file_icon(name));
    let name = gtk::Label::builder().label(fmt::file_name(name)).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["transfer-name"]).build();
    let detail = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label", "numeric"]).build();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
    text.append(&name);
    text.append(&detail);
    let check = gtk::Image::builder().icon_name("object-select-symbolic").css_classes(["dim-label"]).visible(false).build();
    let root = gtk::Box::builder().spacing(12).margin_start(8).margin_end(8).margin_top(8).margin_bottom(8).build();
    root.append(&ring.root);
    root.append(&text);
    root.append(&check);
    (root, FileRow { ring, name, detail, check })
}

#[derive(Clone)]
struct Ring {
    root: gtk::Overlay,
    area: gtk::DrawingArea,
    icon: gtk::Image,
    going: Rc<Cell<bool>>,
    value: Rc<Cell<Option<f64>>>,
}

impl Ring {
    fn new(icon: &gio::Icon) -> Self {
        let area = gtk::DrawingArea::builder().content_width(40).content_height(40).css_classes(["transfer-ring"]).build();
        let image = gtk::Image::builder().gicon(icon).pixel_size(18).halign(gtk::Align::Center).valign(gtk::Align::Center).css_classes(["dim-label"]).build();
        let root = gtk::Overlay::builder().child(&area).valign(gtk::Align::Center).build();
        root.add_overlay(&image);
        let (going, value) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(None)));
        let (g, v) = (going.clone(), value.clone());
        area.set_draw_func(move |area, cr, w, h| {
            let accent = area.color();
            let ink = area.parent().map(|p| p.color()).unwrap_or(accent);
            let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0);
            let tint = if g.get() { (accent, 0.10) } else { (ink, 0.06) };
            cr.set_source_rgba(tint.0.red() as f64, tint.0.green() as f64, tint.0.blue() as f64, tint.1);
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            if !g.get() {
                return;
            }
            let ring = r - 1.5;
            cr.set_line_width(2.5);
            cr.set_source_rgba(ink.red() as f64, ink.green() as f64, ink.blue() as f64, 0.12);
            cr.arc(cx, cy, ring, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            if let Some(v) = v.get() {
                cr.set_line_cap(cairo::LineCap::Round);
                cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 1.0);
                let start = -std::f64::consts::FRAC_PI_2;
                cr.arc(cx, cy, ring, start, start + v * std::f64::consts::TAU);
                let _ = cr.stroke();
            }
        });
        Self { root, area, icon: image, going, value }
    }

    fn set(&self, going: bool, value: Option<f64>) {
        if self.going.get() == going && self.value.get() == value {
            return;
        }
        self.going.set(going);
        self.value.set(value);
        if going {
            self.icon.remove_css_class("dim-label");
            self.icon.add_css_class("transfer-going");
        } else {
            self.icon.remove_css_class("transfer-going");
            self.icon.add_css_class("dim-label");
        }
        self.area.queue_draw();
    }
}

/// A lane with files running along it at the transfer's pace.
#[derive(Clone)]
struct Flow {
    area: gtk::DrawingArea,
    speed: Rc<Cell<f64>>,
}

impl Flow {
    fn new(scanning: bool) -> Self {
        let area = gtk::DrawingArea::builder().content_height(10).hexpand(true).valign(gtk::Align::Center).margin_top(11).margin_bottom(11).css_classes(["flow-lane"]).build();
        let speed = Rc::new(Cell::new(0.0f64));
        let (phase, pace, scan) = (Rc::new(Cell::new(0.0f64)), Rc::new(Cell::new(0.0f64)), Rc::new(Cell::new(0.0f64)));
        let still = mark::reduced_motion(&area);
        let (p, s, sp) = (phase.clone(), scan.clone(), speed.clone());
        area.set_draw_func(move |area, cr, w, h| {
            let c = area.color();
            let (w, h) = (w as f64, h as f64);
            let rgba = |a: f64| cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, a);
            rounded(cr, 0.0, 0.0, w, h, h / 2.0);
            rgba(0.12);
            let _ = cr.fill_preserve();
            cr.clip();
            if scanning {
                let sw = w * 0.3;
                let x = -sw + s.get() * (w + sw);
                let gradient = cairo::LinearGradient::new(x, 0.0, x + sw, 0.0);
                gradient.add_color_stop_rgba(0.0, c.red() as f64, c.green() as f64, c.blue() as f64, 0.0);
                gradient.add_color_stop_rgba(0.5, c.red() as f64, c.green() as f64, c.blue() as f64, 0.55);
                gradient.add_color_stop_rgba(1.0, c.red() as f64, c.green() as f64, c.blue() as f64, 0.0);
                let _ = cr.set_source(&gradient);
                cr.rectangle(x, 0.0, sw, h);
                let _ = cr.fill();
            }
            let phase = match p.get() {
                p if p >= 0.0 => p,
                _ if still && sp.get() > 0.0 => 0.0,
                _ => return,
            };
            const DOTS: usize = 6;
            let (dw, dh) = (14.0, 6.0);
            for i in 0..DOTS {
                let t = (phase + i as f64 / DOTS as f64) % 1.0;
                let x = -dw + t * (w + dw);
                let fade = (t * 8.0).clamp(0.0, 1.0) * ((1.0 - t) * 8.0).clamp(0.0, 1.0);
                rounded(cr, x, (h - dh) / 2.0, dw, dh, 3.0);
                rgba(fade);
                let _ = cr.fill();
            }
        });
        // Still until something moves: a negative phase hides the files
        phase.set(-1.0);
        if !still {
            let last = Rc::new(Cell::new(None::<i64>));
            let target = speed.clone();
            area.add_tick_callback(move |area, clock| {
                let now = clock.frame_time();
                let dt = last.replace(Some(now)).map_or(0.0, |t| (now - t) as f64 / 1e6);
                // 1 MB/s crawls, 100 MB/s races; past that it stays readable
                let mb = target.get() / 1e6;
                let goal = if mb <= 0.0 { 0.0 } else { (0.12 + mb / 80.0).clamp(0.12, 1.6) };
                pace.set(pace.get() + (goal - pace.get()) * (dt * 3.0).clamp(0.0, 1.0));
                if scanning {
                    scan.set((scan.get() + dt / 1.4) % 1.0);
                }
                if pace.get() > 0.001 {
                    phase.set((phase.get().max(0.0) + pace.get() * dt) % 1.0);
                } else if phase.get() >= 0.0 {
                    phase.set(-1.0);
                } else if !scanning {
                    return glib::ControlFlow::Continue;
                }
                area.queue_draw();
                glib::ControlFlow::Continue
            });
        }
        Self { area, speed }
    }
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_done_and_how_far() {
        let sizes = [100, 50, 200];
        assert_eq!(done_files(&sizes, 0), 0);
        assert_eq!(done_files(&sizes, 100), 1);
        assert_eq!(done_files(&sizes, 149), 1);
        assert_eq!(done_files(&sizes, 350), 3);
        assert_eq!(file_fraction(&sizes, 1, 125), Some(0.5));
        assert_eq!(file_fraction(&sizes, 2, 0), Some(0.0));
        assert_eq!(file_fraction(&[0], 0, 0), None);
    }
}
