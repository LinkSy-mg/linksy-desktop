//! A phone's Gallery tab (docs/protocol.md §4.18): its photos and videos, newest first and
//! under a heading for each day, a page at a time as the grid scrolls; each thumbnail is
//! fetched when its square comes into view and decoded off the main thread. Selected ones
//! are saved into the photos folder; double-clicking one saves it and opens it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use linksy_ipc::linksy_core::{Fingerprint, GalleryAlbum, GalleryCursor, GalleryItem};
use linksy_i18n::t;

use super::Window;
use crate::fmt;

const PAGE: u32 = 120;
/// The most a phone gives in one page.
const MAX_PAGE: u32 = 500;
/// The most squares in a row.
const MAX_COLUMNS: usize = 12;

type Fetch = RefCell<Option<Rc<dyn Fn(String)>>>;

pub struct GalleryView {
    pub root: gtk::Box,
    albums: gtk::DropDown,
    album_names: gtk::StringList,
    count: gtk::Label,
    /// Every photo shown, newest first.
    store: gio::ListStore,
    /// What the list shows: the same photos cut into rows under their day's heading.
    rows: gio::ListStore,
    laid: RefCell<Vec<Row>>,
    list: gtk::ListView,
    /// Squares in a row, from the width.
    columns: Cell<usize>,
    /// How tall a square is, from the width.
    square: Cell<i32>,
    selected: RefCell<HashSet<String>>,
    /// Where a range starts (Shift-click).
    anchor: RefCell<Option<String>>,
    scroller: gtk::ScrolledWindow,
    stack: gtk::Stack,
    status: gtk::Label,
    bar: gtk::Box,
    bar_label: gtk::Label,
    save: gtk::Button,
    clear: gtk::Button,
    state: RefCell<GalleryState>,
    /// Thumbnails already on disk, by item id, for the phone shown.
    thumbs: RefCell<HashMap<String, PathBuf>>,
    fetch: Fetch,
    open: Fetch,
    /// Filling the album list, not the user choosing.
    syncing: Cell<bool>,
}

#[derive(Default)]
struct GalleryState {
    phone: Option<Fingerprint>,
    albums: Vec<GalleryAlbum>,
    album: Option<String>,
    more: bool,
    loading: bool,
    /// Bumped when what's shown starts over, so late answers are dropped.
    generation: u64,
    /// The phone's photos changed while another tab showed.
    stale: bool,
}

impl GalleryView {
    pub fn new() -> Rc<Self> {
        let album_names = gtk::StringList::new(&[t!("All Photos")]);
        let albums = gtk::DropDown::builder().model(&album_names).valign(gtk::Align::Center).build();
        let count = gtk::Label::builder().xalign(1.0).hexpand(true).css_classes(["dim-label", "numeric"]).build();
        let top = gtk::Box::builder().spacing(10).build();
        top.append(&albums);
        top.append(&count);

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::builder().model(&gtk::NoSelection::new(Some(rows.clone()))).factory(&factory).css_classes(["gallery-grid"]).build();
        let scroller = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).height_request(600).css_classes(["inner-scroll", "gallery-box"]).build();
        let status = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(40).margin_bottom(40).build();
        let stack = gtk::Stack::builder().vhomogeneous(false).build();
        stack.add_named(&scroller, Some("grid"));
        stack.add_named(&status, Some("status"));

        let bar_label = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["heading"]).build();
        let save = gtk::Button::builder().label(t!("Save to Pictures")).css_classes(["suggested-action"]).build();
        let clear = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text(t!("Clear the Selection")).css_classes(["flat", "circular"]).build();
        let bar = gtk::Box::builder().spacing(8).visible(false).css_classes(["selection-bar"]).build();
        bar.append(&bar_label);
        bar.append(&save);
        bar.append(&clear);

        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        root.append(&top);
        root.append(&stack);
        root.append(&bar);
        let view = Rc::new(Self {
            root,
            albums,
            album_names,
            count,
            store,
            rows,
            laid: RefCell::default(),
            list,
            columns: Cell::new(4),
            square: Cell::new(132),
            selected: RefCell::default(),
            anchor: RefCell::default(),
            scroller,
            stack,
            status,
            bar,
            bar_label,
            save,
            clear,
            state: RefCell::default(),
            thumbs: RefCell::default(),
            fetch: RefCell::default(),
            open: RefCell::default(),
            syncing: Cell::new(false),
        });
        view.cells(&factory);
        view.wire();
        view
    }

    fn cells(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        let view = Rc::downgrade(self);
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            item.set_activatable(false);
            item.set_selectable(false);
            let day = gtk::Label::builder().xalign(0.0).margin_top(14).margin_bottom(4).margin_start(6).css_classes(["heading"]).build();
            let line = gtk::Box::builder().homogeneous(true).build();
            for _ in 0..MAX_COLUMNS {
                line.append(&Self::slot(&view));
            }
            let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            row.append(&day);
            row.append(&line);
            item.set_child(Some(&row));
        });
        let view = Rc::downgrade(self);
        factory.connect_bind(move |_, item| {
            let Some(view) = view.upgrade() else { return };
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let (Some(obj), Some(row)) = (item.item().and_downcast::<glib::BoxedAnyObject>(), item.child().and_downcast::<gtk::Box>()) else { return };
            let (Some(day), Some(line)) = (row.first_child().and_downcast::<gtk::Label>(), row.last_child().and_downcast::<gtk::Box>()) else { return };
            match &*obj.borrow::<Row>() {
                Row::Day(label) => {
                    day.set_label(label);
                    day.set_visible(true);
                    line.set_visible(false);
                }
                Row::Photos(photos) => {
                    day.set_visible(false);
                    line.set_visible(true);
                    let columns = view.columns.get();
                    let mut next = line.first_child();
                    for i in 0..MAX_COLUMNS {
                        let Some(slot) = next.and_downcast::<gtk::Box>() else { break };
                        next = slot.next_sibling();
                        slot.set_visible(i < columns);
                        let Some(cell) = slot.first_child().and_downcast::<gtk::Overlay>() else { continue };
                        match photos.get(i) {
                            Some(g) => {
                                cell.set_visible(true);
                                view.bind_photo(&slot, &cell, g);
                            }
                            None => {
                                cell.set_visible(false);
                                cell.set_widget_name("");
                            }
                        }
                    }
                }
            }
        });
        factory.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(line) = item.child().and_then(|row| row.last_child()) else { return };
            let mut next = line.first_child();
            while let Some(slot) = next {
                next = slot.next_sibling();
                if let Some(cell) = slot.first_child() {
                    cell.set_widget_name("");
                }
            }
        });
    }

    /// One square of a row: the photo, a ring when selected; clicks pick it.
    fn slot(view: &std::rc::Weak<Self>) -> gtk::Box {
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
        let duration = gtk::Label::builder().halign(gtk::Align::End).valign(gtk::Align::End).visible(false).css_classes(["video-badge", "numeric"]).build();
        let cell = gtk::Overlay::builder().child(&picture).overflow(gtk::Overflow::Hidden).css_classes(["gallery-cell"]).build();
        cell.add_overlay(&duration);
        let slot = gtk::Box::builder().hexpand(true).css_classes(["gallery-slot"]).build();
        slot.append(&cell);
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        let view = view.clone();
        click.connect_pressed(move |gesture, presses, _, _| {
            let Some(view) = view.upgrade() else { return };
            let Some(id) = gesture.widget().and_then(|s| s.first_child()).map(|c| c.widget_name().to_string()) else { return };
            view.press(&id, presses, gesture.current_event_state());
        });
        slot.add_controller(click);
        slot
    }

    fn bind_photo(self: &Rc<Self>, slot: &gtk::Box, cell: &gtk::Overlay, g: &GalleryItem) {
        let (Some(picture), Some(duration)) = (cell.child().and_downcast::<gtk::Picture>(), cell.last_child().and_downcast::<gtk::Label>()) else { return };
        picture.set_paintable(gdk::Paintable::NONE);
        picture.set_height_request(self.square.get());
        cell.set_widget_name(&g.id);
        cell.set_tooltip_text(Some(&format!("{}\n{}", g.name, super::page::day_label(g.taken_ms))));
        duration.set_visible(g.video);
        duration.set_label(&g.duration_ms.map(clock).unwrap_or_default());
        if self.selected.borrow().contains(&g.id) {
            slot.add_css_class("selected");
        } else {
            slot.remove_css_class("selected");
        }
        let known = self.thumbs.borrow().get(&g.id).cloned();
        match known {
            Some(path) => show_thumb(cell, &g.id, path),
            None => {
                let fetch = self.fetch.borrow().clone();
                if let Some(fetch) = fetch {
                    fetch(g.id.clone());
                }
            }
        }
    }

    /// The list follows the photos, and the squares follow the width.
    fn wire(self: &Rc<Self>) {
        let view = Rc::downgrade(self);
        self.store.connect_items_changed(move |_, _, _, _| {
            if let Some(view) = view.upgrade() {
                view.relayout();
            }
        });
        let view = Rc::downgrade(self);
        self.list.add_tick_callback(move |list, _| {
            if let Some(view) = view.upgrade() {
                view.fit(list.width());
            }
            glib::ControlFlow::Continue
        });
    }

    /// As many squares as the width has room for.
    fn fit(&self, width: i32) {
        if width <= 0 {
            return;
        }
        let columns = (width / 140).clamp(3, MAX_COLUMNS as i32) as usize;
        let square = width / columns as i32 - 6;
        if square != self.square.get() {
            self.square.set(square);
            for (_, cell) in self.squares() {
                if let Some(picture) = cell.child() {
                    picture.set_height_request(square);
                }
            }
        }
        if columns != self.columns.get() {
            self.columns.set(columns);
            self.relayout();
        }
    }

    /// Cut the photos into rows again, touching only the rows from the first that changed.
    fn relayout(&self) {
        let items = self.items();
        let ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let pruned = {
            let mut selected = self.selected.borrow_mut();
            let before = selected.len();
            selected.retain(|id| ids.contains(id.as_str()));
            selected.len() != before
        };
        let new = rows(&items, self.columns.get(), |ms| section_label(ms));
        let (keep, removed) = {
            let old = self.laid.borrow();
            (old.iter().zip(&new).take_while(|(a, b)| a == b).count(), old.len())
        };
        let added: Vec<glib::BoxedAnyObject> = new[keep..].iter().cloned().map(glib::BoxedAnyObject::new).collect();
        self.rows.splice(keep as u32, (removed - keep) as u32, &added);
        *self.laid.borrow_mut() = new;
        if pruned {
            self.sync_selection();
        }
    }

    /// A click on photo `id`: Ctrl adds or drops it, Shift takes the range from the last
    /// one picked, plain picks only it; a second click opens it.
    fn press(&self, id: &str, presses: i32, state: gdk::ModifierType) {
        if presses >= 2 {
            let open = self.open.borrow().clone();
            if let Some(open) = open {
                open(id.to_string());
            }
            return;
        }
        let shown = self.shown();
        let at = |id: &str| shown.iter().position(|(_, i)| i == id);
        {
            let mut selected = self.selected.borrow_mut();
            let mut anchor = self.anchor.borrow_mut();
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                if !selected.remove(id) {
                    selected.insert(id.to_string());
                }
                *anchor = Some(id.to_string());
            } else if state.contains(gdk::ModifierType::SHIFT_MASK)
                && let (Some(from), Some(to)) = (anchor.as_deref().and_then(at), at(id))
            {
                selected.clear();
                selected.extend(shown[from.min(to)..=from.max(to)].iter().map(|(_, i)| i.clone()));
            } else {
                selected.clear();
                selected.insert(id.to_string());
                *anchor = Some(id.to_string());
            }
        }
        self.sync_selection();
    }

    /// The rings and the bar show what's selected.
    fn sync_selection(&self) {
        let selected = self.selected.borrow();
        for (slot, cell) in self.squares() {
            if selected.contains(cell.widget_name().as_str()) {
                slot.add_css_class("selected");
            } else {
                slot.remove_css_class("selected");
            }
        }
        self.bar.set_visible(!selected.is_empty());
        self.bar_label.set_label(&t!("{what} selected", what = fmt::plural(selected.len(), "item")));
    }

    fn deselect(&self) {
        self.selected.borrow_mut().clear();
        self.sync_selection();
    }

    /// Every square built now, with its photo's cell.
    fn squares(&self) -> Vec<(gtk::Box, gtk::Overlay)> {
        let mut found = Vec::new();
        let mut child = self.list.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            let Some(line) = c.first_child().and_then(|row| row.last_child()) else { continue };
            let mut next = line.first_child();
            while let Some(slot) = next {
                next = slot.next_sibling();
                if let (Some(slot), Some(cell)) = (slot.clone().downcast::<gtk::Box>().ok(), slot.first_child().and_downcast::<gtk::Overlay>()) {
                    found.push((slot, cell));
                }
            }
        }
        found
    }

    fn status(&self, text: &str) {
        self.status.set_label(text);
        self.stack.set_visible_child_name("status");
    }

    /// The cell showing item `id`, if one does.
    fn cell_of(&self, id: &str) -> Option<gtk::Overlay> {
        self.squares().into_iter().map(|(_, cell)| cell).find(|cell| cell.widget_name() == id)
    }

    fn selected_ids(&self) -> Vec<String> {
        let selected = self.selected.borrow();
        self.shown().into_iter().map(|(_, id)| id).filter(|id| selected.contains(id)).collect()
    }

    fn items(&self) -> Vec<GalleryItem> {
        (0..self.store.n_items()).filter_map(|i| self.store.item(i).and_downcast::<glib::BoxedAnyObject>()).map(|o| o.borrow::<GalleryItem>().clone()).collect()
    }

    /// What the grid shows, in order: each item's time and id.
    fn shown(&self) -> Vec<(u64, String)> {
        (0..self.store.n_items())
            .filter_map(|i| self.store.item(i).and_downcast::<glib::BoxedAnyObject>())
            .map(|o| {
                let i = o.borrow::<GalleryItem>();
                (i.taken_ms, i.id.clone())
            })
            .collect()
    }

    fn last_cursor(&self) -> Option<GalleryCursor> {
        let n = self.store.n_items();
        let last = self.store.item(n.checked_sub(1)?).and_downcast::<glib::BoxedAnyObject>()?;
        let g = last.borrow::<GalleryItem>();
        Some(GalleryCursor { taken_ms: g.taken_ms, id: g.id.clone() })
    }
}

/// Decode a thumbnail off the main thread, then show it if the cell still shows that item.
fn show_thumb(cell: &gtk::Overlay, id: &str, path: PathBuf) {
    let (cell, id) = (cell.clone(), id.to_string());
    glib::spawn_future_local(async move {
        let decoded = gio::spawn_blocking(move || {
            let p = gtk::gdk_pixbuf::Pixbuf::from_file(&path).ok()?;
            Some((p.width(), p.height(), p.rowstride() as usize, p.has_alpha(), p.read_pixel_bytes()))
        })
        .await
        .ok()
        .flatten();
        if let Some((width, height, stride, alpha, pixels)) = decoded
            && cell.widget_name() == id
            && let Some(picture) = cell.child().and_downcast::<gtk::Picture>()
        {
            let format = if alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
            picture.set_paintable(Some(&gdk::MemoryTexture::new(width, height, format, &pixels, stride)));
        }
    });
}

/// A row of the list: a day's heading, or up to a row's worth of that day's photos.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Day(String),
    Photos(Vec<GalleryItem>),
}

/// Photos (newest first) cut into rows of `columns` under a heading for each day, as
/// `label` names the day of a time.
fn rows(items: &[GalleryItem], columns: usize, label: impl Fn(u64) -> String) -> Vec<Row> {
    let mut out = Vec::new();
    let mut day: Option<String> = None;
    for item in items {
        let name = label(item.taken_ms);
        if day.as_ref() != Some(&name) {
            out.push(Row::Day(name.clone()));
            day = Some(name);
        }
        match out.last_mut() {
            Some(Row::Photos(row)) if row.len() < columns => row.push(item.clone()),
            _ => out.push(Row::Photos(vec![item.clone()])),
        }
    }
    out
}

/// "Today", "Yesterday", "Nov 2"; with the year for earlier years, so days never merge.
fn section_label(at_ms: u64) -> String {
    super::page::day_heading(at_ms, |this_year| if this_year { "%b %e" } else { "%b %e, %Y" })
}

#[derive(Debug, PartialEq)]
enum Change {
    Remove(usize),
    Insert(usize, GalleryItem),
}

/// How to bring what's shown (newest first) in line with the newest page from the phone,
/// changes applied in order. Shown items within the page's span but not in it were
/// deleted; items older than the span stay, as does anything when the page holds all.
fn reconcile(shown: &[(u64, String)], fresh: Vec<GalleryItem>, more: bool) -> Vec<Change> {
    // Ties at the span's oldest millisecond are left alone: the phone's order within one
    // isn't known here
    let oldest = fresh.last().map(|i| i.taken_ms).filter(|_| more);
    let ids: HashSet<&str> = fresh.iter().map(|i| i.id.as_str()).collect();
    let mut changes = Vec::new();
    let mut kept = Vec::with_capacity(shown.len());
    for (i, (taken, id)) in shown.iter().enumerate().rev() {
        if oldest.is_none_or(|o| *taken > o) && !ids.contains(id.as_str()) {
            changes.push(Change::Remove(i));
        } else {
            kept.push(id.clone());
        }
    }
    kept.reverse();
    // The page's order is the phone's: each new one goes right after the last one shown
    // before it in the page
    let mut at = 0;
    for item in fresh {
        if let Some(i) = kept[at..].iter().position(|id| *id == item.id) {
            at += i + 1;
        } else {
            kept.insert(at, item.id.clone());
            changes.push(Change::Insert(at, item));
            at += 1;
        }
    }
    changes
}

/// "0:14", "12:03", "1:02:03".
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

impl Window {
    pub(super) fn setup_gallery(self: &Rc<Self>) {
        let g = &self.gallery;
        let w = Rc::downgrade(self);
        *g.fetch.borrow_mut() = Some(Rc::new(move |id| {
            if let Some(w) = w.upgrade() {
                w.gallery_thumb(id);
            }
        }));
        let w = Rc::downgrade(self);
        g.albums.connect_selected_notify(move |d| {
            let Some(w) = w.upgrade() else { return };
            if w.gallery.syncing.get() {
                return;
            }
            let album = d.selected().checked_sub(1).and_then(|i| w.gallery.state.borrow().albums.get(i as usize).map(|a| a.id.clone()));
            w.gallery.state.borrow_mut().album = album;
            w.gallery_page(true);
        });
        let w = Rc::downgrade(self);
        g.scroller.vadjustment().connect_value_changed(move |a| {
            if let Some(w) = w.upgrade()
                && a.value() + a.page_size() > a.upper() - 400.0
            {
                w.gallery_page(false);
            }
        });
        let w = Rc::downgrade(self);
        g.save.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                let ids = w.gallery.selected_ids();
                w.gallery_save(ids, false);
                w.gallery.deselect();
            }
        });
        let w = Rc::downgrade(self);
        g.clear.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.gallery.deselect();
            }
        });
        let w = Rc::downgrade(self);
        *g.open.borrow_mut() = Some(Rc::new(move |id| {
            if let Some(w) = w.upgrade() {
                w.gallery_save(vec![id], true);
            }
        }));
    }

    /// The Gallery tab shows: the phone's albums and newest photos the first time, brought
    /// up to date when they changed meanwhile.
    pub(super) fn gallery_shown(self: &Rc<Self>, phone: Fingerprint) {
        if self.gallery.state.borrow().phone == Some(phone) && self.gallery.store.n_items() > 0 {
            let stale = std::mem::take(&mut self.gallery.state.borrow_mut().stale);
            if stale {
                self.gallery_albums(phone);
                self.gallery_refresh(phone);
            }
            return;
        }
        let generation = self.gallery.state.borrow().generation + 1;
        *self.gallery.state.borrow_mut() = GalleryState { phone: Some(phone), generation, ..Default::default() };
        self.gallery.thumbs.borrow_mut().clear();
        self.gallery.store.remove_all();
        self.gallery.count.set_label("");
        self.gallery_albums(phone);
        self.gallery_page(true);
    }

    /// The phone's photos changed (taken, deleted…): the grid follows, now or when the tab
    /// shows again, if its gallery is loaded.
    pub(super) fn gallery_changed(self: &Rc<Self>, phone: &Fingerprint) {
        if self.gallery.state.borrow().phone != Some(*phone) {
            return;
        }
        if self.page.tab() == super::page::Tab::Gallery {
            self.gallery_albums(*phone);
            self.gallery_refresh(*phone);
        } else {
            self.gallery.state.borrow_mut().stale = true;
        }
    }

    fn gallery_albums(self: &Rc<Self>, phone: Fingerprint) {
        let Some(c) = self.client() else { return };
        let call = self.bridge.call(&c, move |c| async move { c.gallery_albums(phone.to_string()).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let Ok(albums) = call.await else { return };
            if w.gallery.state.borrow().phone != Some(phone) {
                return;
            }
            let g = &w.gallery;
            let chosen = g.state.borrow().album.clone();
            g.syncing.set(true);
            let mut names: Vec<String> = vec![t!("All Photos").into()];
            names.extend(albums.iter().map(|a| format!("{} ({})", a.name, a.count)));
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            g.album_names.splice(0, g.album_names.n_items(), &names);
            let at = chosen.and_then(|id| albums.iter().position(|a| a.id == id)).map_or(0, |i| i + 1);
            g.albums.set_selected(at as u32);
            let total: u64 = albums.iter().map(|a| a.count).sum();
            g.count.set_label(&fmt::plural(total as usize, "item"));
            g.state.borrow_mut().albums = albums;
            g.syncing.set(false);
        });
    }

    /// The first page (`fresh`), or the next one when there's more and none is coming.
    fn gallery_page(self: &Rc<Self>, fresh: bool) {
        let g = &self.gallery;
        let (Some(c), Some(phone)) = (self.client(), g.state.borrow().phone) else { return };
        let before = if fresh {
            let mut state = g.state.borrow_mut();
            state.generation += 1;
            state.loading = false;
            None
        } else {
            let state = g.state.borrow();
            if state.loading || !state.more {
                return;
            }
            drop(state);
            g.last_cursor()
        };
        if fresh {
            g.store.remove_all();
            g.deselect();
            g.status(t!("Loading…"));
        }
        let (album, generation) = {
            let mut state = g.state.borrow_mut();
            state.loading = true;
            (state.album.clone(), state.generation)
        };
        let call = self.bridge.call(&c, move |c| async move { c.gallery_items(phone.to_string(), album, before, PAGE).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            let g = &w.gallery;
            if g.state.borrow().generation != generation {
                return;
            }
            g.state.borrow_mut().loading = false;
            match result {
                Ok(page) => {
                    g.state.borrow_mut().more = page.more;
                    // A refresh may have put some of these in already
                    let shown: HashSet<String> = g.shown().into_iter().map(|(_, id)| id).collect();
                    let objects: Vec<glib::BoxedAnyObject> = page.items.into_iter().filter(|i| !shown.contains(&i.id)).map(glib::BoxedAnyObject::new).collect();
                    g.store.splice(g.store.n_items(), 0, &objects);
                    if g.store.n_items() == 0 {
                        g.status(t!("No photos here yet."));
                    } else {
                        g.stack.set_visible_child_name("grid");
                        // Short pages leave nothing to scroll: ask for the next one now.
                        let a = g.scroller.vadjustment();
                        if page.more && a.upper() <= a.page_size() + 400.0 {
                            let w = w.clone();
                            glib::idle_add_local_once(move || w.gallery_page(false));
                        }
                    }
                }
                Err(e) if g.store.n_items() == 0 => g.status(&fmt::capitalize(&e)),
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }

    /// The newest photos again, as many as shown: deleted ones out, new ones in, where
    /// they go. The scroll position and the selection stay.
    fn gallery_refresh(self: &Rc<Self>, phone: Fingerprint) {
        let Some(c) = self.client() else { return };
        let (album, generation) = {
            let state = self.gallery.state.borrow();
            (state.album.clone(), state.generation)
        };
        let limit = self.gallery.store.n_items().clamp(PAGE, MAX_PAGE);
        let call = self.bridge.call(&c, move |c| async move { c.gallery_items(phone.to_string(), album, None, limit).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let Ok(page) = call.await else { return };
            let g = &w.gallery;
            if g.state.borrow().generation != generation {
                return;
            }
            if !page.more {
                g.state.borrow_mut().more = false;
            }
            for change in reconcile(&g.shown(), page.items, page.more) {
                match change {
                    Change::Remove(at) => g.store.remove(at as u32),
                    Change::Insert(at, item) => g.store.insert(at as u32, &glib::BoxedAnyObject::new(item)),
                }
            }
            if g.store.n_items() == 0 {
                g.status(t!("No photos here yet."));
            } else {
                g.stack.set_visible_child_name("grid");
            }
        });
    }

    fn gallery_thumb(self: &Rc<Self>, id: String) {
        let (Some(c), Some(phone)) = (self.client(), self.gallery.state.borrow().phone) else { return };
        let item = id.clone();
        let call = self.bridge.call(&c, move |c| async move { c.gallery_thumb(phone.to_string(), item).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let Ok(path) = call.await else { return };
            if w.gallery.state.borrow().phone != Some(phone) {
                return;
            }
            w.gallery.thumbs.borrow_mut().insert(id.clone(), path.clone());
            if let Some(cell) = w.gallery.cell_of(&id) {
                show_thumb(&cell, &id, path);
            }
        });
    }

    /// Have the phone send these into the photos folder; `open`: open it once it's here.
    fn gallery_save(self: &Rc<Self>, ids: Vec<String>, open: bool) {
        let (Some(c), Some(phone)) = (self.client(), self.gallery.state.borrow().phone) else { return };
        if ids.is_empty() {
            return;
        }
        let n = ids.len();
        let call = self.bridge.call(&c, move |c| async move { c.gallery_save(phone.to_string(), ids).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(transfer) if open => {
                    w.state.borrow_mut().open_when_done.insert(transfer);
                }
                Ok(_) => w.toast(&t!("Saving {what} to your photos folder", what = fmt::plural(n, "item"))),
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, taken_ms: u64) -> GalleryItem {
        GalleryItem { id: id.into(), name: format!("{id}.jpg"), video: false, taken_ms, size: 1, duration_ms: None }
    }

    fn shown(items: &[(&str, u64)]) -> Vec<(u64, String)> {
        items.iter().map(|(id, t)| (*t, id.to_string())).collect()
    }

    /// The list after the changes, to compare with what the phone has.
    fn apply(shown: &[(u64, String)], changes: Vec<Change>) -> Vec<String> {
        let mut ids: Vec<String> = shown.iter().map(|(_, id)| id.clone()).collect();
        for c in changes {
            match c {
                Change::Remove(at) => {
                    ids.remove(at);
                }
                Change::Insert(at, item) => ids.insert(at, item.id),
            }
        }
        ids
    }

    /// The day of a time as tests name it: a time is `day * 100 + n`.
    fn day_of(ms: u64) -> String {
        format!("day {}", ms / 100)
    }

    fn ids(row: &Row) -> Vec<&str> {
        match row {
            Row::Photos(p) => p.iter().map(|i| i.id.as_str()).collect(),
            Row::Day(d) => vec![d.as_str()],
        }
    }

    #[test]
    fn each_day_gets_a_heading_and_rows_fill_up() {
        let items = [item("a", 305), item("b", 304), item("c", 303), item("d", 202), item("e", 101)];
        let laid = rows(&items, 2, day_of);
        let flat: Vec<Vec<&str>> = laid.iter().map(ids).collect();
        assert_eq!(flat, vec![vec!["day 3"], vec!["a", "b"], vec!["c"], vec!["day 2"], vec!["d"], vec!["day 1"], vec!["e"]]);
    }

    #[test]
    fn a_new_day_never_shares_a_row() {
        let items = [item("a", 201), item("b", 101)];
        assert_eq!(rows(&items, 4, day_of).len(), 4);
    }

    #[test]
    fn no_photos_no_rows() {
        assert!(rows(&[], 4, day_of).is_empty());
    }

    #[test]
    fn narrower_rows_split_a_day_further() {
        let items: Vec<GalleryItem> = (0..7).map(|n| item(&format!("p{n}"), 350 - n)).collect();
        let wide = rows(&items, 7, day_of);
        let narrow = rows(&items, 3, day_of);
        assert_eq!(wide.len(), 2);
        assert_eq!(narrow.len(), 4);
    }

    #[test]
    fn deleted_ones_go() {
        let s = shown(&[("e", 50), ("d", 40), ("c", 30), ("b", 20), ("a", 10)]);
        let changes = reconcile(&s, vec![item("e", 50), item("c", 30), item("a", 10)], false);
        assert_eq!(changes, vec![Change::Remove(3), Change::Remove(1)]);
        assert_eq!(apply(&s, changes), ["e", "c", "a"]);
    }

    #[test]
    fn new_ones_come_in_where_they_go() {
        let s = shown(&[("e", 50), ("c", 30), ("a", 10)]);
        let fresh = vec![item("f", 60), item("e", 50), item("d", 40), item("c", 30), item("a", 10)];
        assert_eq!(apply(&s, reconcile(&s, fresh, false)), ["f", "e", "d", "c", "a"]);
    }

    #[test]
    fn older_than_the_page_stays() {
        // Two pages shown, the newest asked again: the second page isn't touched
        let s = shown(&[("e", 50), ("d", 40), ("c", 30), ("b", 20), ("a", 10)]);
        let fresh = vec![item("e", 50), item("c", 30)];
        assert_eq!(apply(&s, reconcile(&s, fresh, true)), ["e", "c", "b", "a"]);
    }

    #[test]
    fn deletions_pull_older_ones_up() {
        // Two deleted in front: the page reaches past what's shown, and those follow on
        let s = shown(&[("e", 50), ("d", 40), ("c", 30)]);
        let fresh = vec![item("e", 50), item("b", 20), item("a", 10)];
        assert_eq!(apply(&s, reconcile(&s, fresh, true)), ["e", "b", "a"]);
    }

    #[test]
    fn deleted_and_added_at_once() {
        let s = shown(&[("e", 50), ("d", 40), ("c", 30)]);
        let fresh = vec![item("g", 70), item("e", 50), item("c", 30)];
        assert_eq!(apply(&s, reconcile(&s, fresh, false)), ["g", "e", "c"]);
    }

    #[test]
    fn all_deleted() {
        let s = shown(&[("b", 20), ("a", 10)]);
        assert_eq!(apply(&s, reconcile(&s, vec![], false)), Vec::<String>::new());
    }

    #[test]
    fn same_millisecond_at_the_edge_stays() {
        let s = shown(&[("c", 30), ("b", 20), ("a", 20)]);
        let fresh = vec![item("c", 30), item("b", 20)];
        assert_eq!(apply(&s, reconcile(&s, fresh, true)), ["c", "b", "a"]);
    }

    #[test]
    fn nothing_changed_nothing_to_do() {
        let s = shown(&[("c", 30), ("b", 20)]);
        assert!(reconcile(&s, vec![item("c", 30), item("b", 20)], true).is_empty());
    }
}
