//! A phone's Gallery tab (docs/protocol.md §4.18): its photos and videos, newest first, a
//! page at a time as the grid scrolls; each thumbnail is fetched when its square comes into
//! view and decoded off the main thread. Selected ones are saved into the photos folder;
//! double-clicking one saves it and opens it.

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

type Fetch = RefCell<Option<Rc<dyn Fn(String)>>>;

pub struct GalleryView {
    pub root: gtk::Box,
    albums: gtk::DropDown,
    album_names: gtk::StringList,
    count: gtk::Label,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    grid: gtk::GridView,
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
        let selection = gtk::MultiSelection::new(Some(store.clone()));
        let factory = gtk::SignalListItemFactory::new();
        let grid = gtk::GridView::builder().model(&selection).factory(&factory).min_columns(3).max_columns(12).enable_rubberband(true).css_classes(["gallery-grid"]).build();
        let scroller = gtk::ScrolledWindow::builder().child(&grid).hscrollbar_policy(gtk::PolicyType::Never).height_request(600).css_classes(["inner-scroll", "gallery-box"]).build();
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
            selection,
            grid,
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
            syncing: Cell::new(false),
        });
        view.cells(&factory);
        view
    }

    fn cells(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).width_request(132).height_request(132).build();
            let duration = gtk::Label::builder().halign(gtk::Align::End).valign(gtk::Align::End).visible(false).css_classes(["video-badge", "numeric"]).build();
            let cell = gtk::Overlay::builder().child(&picture).overflow(gtk::Overflow::Hidden).css_classes(["gallery-cell"]).build();
            cell.add_overlay(&duration);
            item.set_child(Some(&cell));
        });
        let view = Rc::downgrade(self);
        factory.connect_bind(move |_, item| {
            let Some(view) = view.upgrade() else { return };
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let (Some(obj), Some(cell)) = (item.item().and_downcast::<glib::BoxedAnyObject>(), item.child().and_downcast::<gtk::Overlay>()) else { return };
            let g = obj.borrow::<GalleryItem>();
            let picture = cell.child().and_downcast::<gtk::Picture>().expect("picture");
            let duration = cell.last_child().and_downcast::<gtk::Label>().expect("duration");
            picture.set_paintable(gdk::Paintable::NONE);
            cell.set_widget_name(&g.id);
            cell.set_tooltip_text(Some(&format!("{}\n{}", g.name, super::page::day_label(g.taken_ms))));
            duration.set_visible(g.video);
            duration.set_label(&g.duration_ms.map(clock).unwrap_or_default());
            let known = view.thumbs.borrow().get(&g.id).cloned();
            match known {
                Some(path) => show_thumb(&cell, &g.id, path),
                None => {
                    let fetch = view.fetch.borrow().clone();
                    if let Some(fetch) = fetch {
                        fetch(g.id.clone());
                    }
                }
            }
        });
        factory.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            if let Some(cell) = item.child() {
                cell.set_widget_name("");
            }
        });
    }

    fn status(&self, text: &str) {
        self.status.set_label(text);
        self.stack.set_visible_child_name("status");
    }

    /// The cell showing item `id`, if one does.
    fn cell_of(&self, id: &str) -> Option<gtk::Overlay> {
        let mut child = self.grid.first_child();
        while let Some(c) = child {
            if let Some(cell) = c.first_child().and_downcast::<gtk::Overlay>()
                && cell.widget_name() == id
            {
                return Some(cell);
            }
            child = c.next_sibling();
        }
        None
    }

    fn selected_ids(&self) -> Vec<String> {
        let set = self.selection.selection();
        (0..set.size()).filter_map(|i| self.store.item(set.nth(i as u32)).and_downcast::<glib::BoxedAnyObject>()).map(|o| o.borrow::<GalleryItem>().id.clone()).collect()
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
        g.selection.connect_selection_changed(move |_, _, _| {
            if let Some(w) = w.upgrade() {
                let n = w.gallery.selection.selection().size() as usize;
                w.gallery.bar.set_visible(n > 0);
                w.gallery.bar_label.set_label(&t!("{what} selected", what = fmt::plural(n, "item")));
            }
        });
        let w = Rc::downgrade(self);
        g.save.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                let ids = w.gallery.selected_ids();
                w.gallery_save(ids, false);
                w.gallery.selection.unselect_all();
            }
        });
        let w = Rc::downgrade(self);
        g.clear.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.gallery.selection.unselect_all();
            }
        });
        let w = Rc::downgrade(self);
        g.grid.connect_activate(move |_, position| {
            let Some(w) = w.upgrade() else { return };
            if let Some(obj) = w.gallery.store.item(position).and_downcast::<glib::BoxedAnyObject>() {
                let id = obj.borrow::<GalleryItem>().id.clone();
                w.gallery_save(vec![id], true);
            }
        });
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
            g.selection.unselect_all();
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
