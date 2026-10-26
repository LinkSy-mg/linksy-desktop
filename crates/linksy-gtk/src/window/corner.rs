//! Transfers in progress in a panel at the bottom right of the window, on every page.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use linksy_ipc::linksy_core::{Direction, Uuid};
use linksy_ipc::{HistoryEntry, HistoryStatus, Shared};
use linksy_i18n::t;

use super::{Progress, page::Action, progress_text};
use crate::{fmt, mark};

const LINGER: Duration = Duration::from_secs(8);

type Handler = RefCell<Option<Rc<dyn Fn(Action)>>>;

struct Row {
    id: Uuid,
    root: gtk::Box,
    peer: String,
    detail: gtk::Label,
    bar: gtk::ProgressBar,
    actions: gtk::Box,
    running: bool,
    bytes: u64,
    total: u64,
}

pub(super) struct Corner {
    pub root: gtk::Revealer,
    list: gtk::Box,
    details: gtk::Revealer,
    overall: gtk::ProgressBar,
    fold: gtk::Button,
    close: gtk::Button,
    rows: RefCell<Vec<Row>>,
    folded: Cell<bool>,
    hide: RefCell<Option<glib::SourceId>>,
    handler: Handler,
    title: gtk::Button,
}

impl Corner {
    pub fn new() -> Rc<Self> {
        let title = gtk::Button::builder().label(t!("Transfers")).css_classes(["flat", "corner-title"]).build();
        let overall = gtk::ProgressBar::builder().hexpand(true).valign(gtk::Align::Center).visible(false).css_classes(["transfer-progress"]).build();
        let fold = gtk::Button::builder().icon_name("pan-down-symbolic").tooltip_text(t!("Show Less")).css_classes(["flat", "circular"]).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text(t!("Close")).css_classes(["flat", "circular"]).visible(false).build();
        let head = gtk::Box::builder().spacing(8).build();
        head.append(&title);
        head.append(&overall);
        head.append(&gtk::Box::builder().hexpand(true).build());
        head.append(&fold);
        head.append(&close);

        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_top(10).build();
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).max_content_height(320).child(&list).build();
        let details = gtk::Revealer::builder().reveal_child(true).transition_type(gtk::RevealerTransitionType::SlideDown).child(&scroll).build();

        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(320).css_classes(["card", "corner-panel"]).build();
        card.append(&head);
        card.append(&details);
        card.update_property(&[gtk::accessible::Property::Label(t!("Transfers"))]);
        let root = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(12)
            .margin_bottom(12)
            .margin_start(12)
            .child(&card)
            .build();
        root.set_can_target(false);

        let corner = Rc::new(Self { root, list, details, overall, fold, close, rows: RefCell::default(), folded: Cell::new(false), hide: RefCell::default(), handler: RefCell::default(), title });
        let c = Rc::downgrade(&corner);
        corner.fold.connect_clicked(move |_| {
            if let Some(c) = c.upgrade() {
                c.set_folded(!c.folded.get());
            }
        });
        let c = Rc::downgrade(&corner);
        corner.close.connect_clicked(move |_| {
            if let Some(c) = c.upgrade() {
                c.dismiss();
            }
        });
        corner
    }

    pub fn set_handler(&self, f: impl Fn(Action) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_open(&self, f: impl Fn() + 'static) {
        self.title.connect_clicked(move |_| f());
    }

    pub fn entry(self: &Rc<Self>, e: &HistoryEntry) {
        let Shared::Files { files, total, saved, import, .. } = &e.what else { return };
        let running = matches!(e.status, HistoryStatus::InProgress);
        let known = self.rows.borrow().iter().any(|r| r.id == e.id);
        if !known && !running {
            return;
        }
        if !known {
            self.add(e, files, *import, *total);
        }
        if !running {
            self.finish(e, *total, saved, *import);
        }
        self.refresh();
    }

    pub fn progress(&self, id: Uuid, p: &Progress) {
        let mut rows = self.rows.borrow_mut();
        let Some(row) = rows.iter_mut().find(|r| r.id == id && r.running) else { return };
        row.bytes = p.bytes;
        row.total = p.total;
        row.detail.set_label(&format!("{} · {}", row.peer, progress_text(p)));
        mark::ease_progress(&row.bar, if p.total == 0 { 1.0 } else { p.bytes as f64 / p.total as f64 });
        drop(rows);
        self.update_overall();
    }

    fn add(self: &Rc<Self>, e: &HistoryEntry, files: &[String], import: bool, total: u64) {
        let incoming = e.direction == Direction::Incoming;
        let icon = match (import, incoming) {
            (true, _) => "image-x-generic-symbolic",
            (false, true) => "folder-download-symbolic",
            (false, false) => "document-send-symbolic",
        };
        let title = match (import, files) {
            (true, _) => fmt::plural(files.len(), "photo"),
            (false, [one]) => fmt::file_name(one),
            (false, many) => fmt::plural(many.len(), "item"),
        };
        let node = gtk::Image::builder().icon_name(icon).pixel_size(14).valign(gtk::Align::Start).css_classes(["tl-node", "live"]).build();
        let title = gtk::Label::builder().label(&title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["heading"]).build();
        let waiting = if incoming { t!("Starting…") } else { t!("Waiting for them to accept…") };
        let detail = gtk::Label::builder().label(format!("{} · {waiting}", e.peer_name)).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label", "numeric"]).build();
        let bar = gtk::ProgressBar::builder().margin_top(4).css_classes(["transfer-progress"]).build();
        let actions = gtk::Box::builder().spacing(2).valign(gtk::Align::Start).build();
        let id = e.id;
        actions.append(&self.button("window-close-symbolic", t!("Cancel"), Action::Cancel(id)));
        let head = gtk::Box::builder().spacing(6).build();
        head.append(&title);
        head.append(&actions);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        body.append(&head);
        body.append(&detail);
        body.append(&bar);
        let root = gtk::Box::builder().spacing(10).build();
        root.append(&node);
        root.append(&body);
        self.list.prepend(&root);
        self.rows.borrow_mut().push(Row { id, root, peer: e.peer_name.clone(), detail, bar, actions, running: true, bytes: 0, total });
    }

    fn finish(&self, e: &HistoryEntry, total: u64, saved: &[PathBuf], import: bool) {
        let incoming = e.direction == Direction::Incoming;
        let mut rows = self.rows.borrow_mut();
        let Some(row) = rows.iter_mut().find(|r| r.id == e.id) else { return };
        row.running = false;
        let state = match &e.status {
            HistoryStatus::Done if import => t!("Imported").to_string(),
            HistoryStatus::Done if incoming => t!("Received").to_string(),
            HistoryStatus::Done => t!("Sent").to_string(),
            HistoryStatus::Declined => t!("Declined").to_string(),
            HistoryStatus::Cancelled => t!("Cancelled").to_string(),
            HistoryStatus::Failed { reason } => t!("Failed: {reason}", reason),
            HistoryStatus::InProgress => return,
        };
        let done = matches!(e.status, HistoryStatus::Done);
        row.detail.set_label(&[row.peer.clone(), state, fmt::size(total)].join(" · "));
        row.bar.set_visible(false);
        if let Some(node) = row.root.first_child() {
            node.remove_css_class("live");
            node.add_css_class(if !done { "failed" } else if incoming { "incoming" } else { "outgoing" });
        }
        while let Some(child) = row.actions.first_child() {
            row.actions.remove(&child);
        }
        if done && incoming && !saved.is_empty() {
            let open = self.button("", t!("Open"), Action::Open(saved.to_vec()));
            open.set_label(t!("Open"));
            open.set_css_classes(&["flat", "tl-action"]);
            open.set_tooltip_text(None);
            row.actions.append(&open);
            row.actions.append(&self.button("folder-open-symbolic", t!("Show in Folder"), Action::ShowInFolder(saved[0].clone())));
        }
    }

    fn button(&self, icon: &str, tooltip: &str, action: Action) -> gtk::Button {
        let button = gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).valign(gtk::Align::Center).css_classes(["flat", "circular"]).build();
        let handler = self.handler.borrow().clone();
        button.connect_clicked(move |_| {
            if let Some(f) = &handler {
                f(action.clone());
            }
        });
        button
    }

    fn refresh(self: &Rc<Self>) {
        let running = self.rows.borrow().iter().any(|r| r.running);
        if let Some(id) = self.hide.borrow_mut().take() {
            id.remove();
        }
        if running {
            self.root.set_can_target(true);
            self.root.set_reveal_child(true);
        } else {
            let c = Rc::downgrade(self);
            let id = glib::timeout_add_local_once(LINGER, move || {
                if let Some(c) = c.upgrade() {
                    c.hide.borrow_mut().take();
                    c.dismiss();
                }
            });
            *self.hide.borrow_mut() = Some(id);
        }
        self.close.set_visible(!running);
        self.update_overall();
    }

    fn dismiss(&self) {
        if let Some(id) = self.hide.borrow_mut().take() {
            id.remove();
        }
        let mut rows = self.rows.borrow_mut();
        rows.retain(|r| {
            if !r.running {
                self.list.remove(&r.root);
            }
            r.running
        });
        if rows.is_empty() {
            self.root.set_reveal_child(false);
            self.root.set_can_target(false);
        }
    }

    pub fn reset(&self) {
        if let Some(id) = self.hide.borrow_mut().take() {
            id.remove();
        }
        for row in self.rows.borrow_mut().drain(..) {
            self.list.remove(&row.root);
        }
        self.root.set_reveal_child(false);
        self.root.set_can_target(false);
    }

    fn set_folded(&self, folded: bool) {
        self.folded.set(folded);
        self.details.set_reveal_child(!folded);
        self.fold.set_icon_name(if folded { "pan-up-symbolic" } else { "pan-down-symbolic" });
        self.fold.set_tooltip_text(Some(if folded { t!("Show More") } else { t!("Show Less") }));
        self.update_overall();
    }

    fn update_overall(&self) {
        let rows = self.rows.borrow();
        let (bytes, total) = rows.iter().filter(|r| r.running).fold((0, 0), |(b, t), r| (b + r.bytes, t + r.total));
        self.overall.set_visible(self.folded.get() && rows.iter().any(|r| r.running));
        if total > 0 {
            self.overall.set_fraction(bytes as f64 / total as f64);
        }
    }
}

