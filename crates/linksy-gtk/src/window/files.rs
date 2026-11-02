//! A phone's Files tab: a file manager for the folder it shares, one folder at a time. The
//! list grows down the page, 200 rows at a time (a camera folder holds thousands of photos).
//! A click picks a
//! row (Ctrl and Shift add to it, as in Files); a double click or Enter opens it: a folder
//! here, a file through the same bridge as the Files sidebar. Copies to and from the phone
//! run in the service and show under the list while they go.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use linksy_ipc::FsProgress;
use linksy_ipc::linksy_core::{Fingerprint, FsEntry, Uuid};
use linksy_i18n::t;

use super::Window;
use crate::fmt;

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Name,
    Date,
    Size,
}

/// A menu's choices, one picked at a time.
type Choices<T> = Vec<(T, gtk::CheckButton)>;

/// Rows built at first, and added each time the page scrolls to their end.
const ROWS: u32 = 200;

/// A row's entry, with what sorting and searching compare.
struct Item {
    entry: FsEntry,
    key: glib::FilenameCollationKey,
    lower: String,
}

fn new_item(entry: FsEntry) -> glib::BoxedAnyObject {
    glib::BoxedAnyObject::new(Item { key: entry.name.as_str().into(), lower: entry.name.to_lowercase(), entry })
}

fn item(obj: &glib::Object) -> Ref<'_, Item> {
    obj.downcast_ref::<glib::BoxedAnyObject>().expect("a file item").borrow()
}

/// A copy going on, under the list.
struct CopyRow {
    phone: Fingerprint,
    root: gtk::Box,
    title: gtk::Label,
    detail: gtk::Label,
    bar: gtk::ProgressBar,
}

pub struct FilesView {
    pub root: gtk::Box,
    crumbs: gtk::Box,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    sorter: gtk::CustomSorter,
    sorted: gtk::SortListModel,
    /// The rows built: the first ones, more as the page scrolls to their end.
    shown: gtk::SliceListModel,
    selection: gtk::MultiSelection,
    list: gtk::ListView,
    stack: gtk::Stack,
    status: gtk::Label,
    search_toggle: gtk::ToggleButton,
    search_bar: gtk::Box,
    search: gtk::SearchEntry,
    query: Rc<RefCell<String>>,
    sort: Rc<Cell<(SortBy, bool)>>,
    sorts: Choices<SortBy>,
    orders: Choices<bool>,
    new_folder: gtk::Button,
    open_folder: gtk::Button,
    picked_bar: gtk::Box,
    picked: gtk::Label,
    copy: gtk::Button,
    rename: gtk::Button,
    delete: gtk::Button,
    clear: gtk::Button,
    /// Buttons that show only their icon on a narrow page: icon, label.
    labelled: Vec<(gtk::Button, &'static str, &'static str)>,
    copies: gtk::Box,
    rows: RefCell<HashMap<Uuid, CopyRow>>,
    /// The listing asked for last, until it comes.
    loading: Cell<Option<u64>>,
    loads: Cell<u64>,
    state: RefCell<FilesState>,
}

#[derive(Default)]
struct FilesState {
    phone: Option<Fingerprint>,
    /// The folder shown, "/" first, always ending in "/".
    path: String,
    /// Where apps open the phone's files, once asked.
    place: Option<String>,
}

impl FilesView {
    pub fn new() -> Rc<Self> {
        let crumbs = gtk::Box::builder().spacing(2).hexpand(true).build();
        let tool = |icon: &str, tip: &str| gtk::Button::builder().icon_name(icon).tooltip_text(tip).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let search_toggle = gtk::ToggleButton::builder().icon_name("system-search-symbolic").tooltip_text(t!("Search")).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let (sort_menu, sorts, orders) = sort_menu();
        let new_folder = tool("folder-new-symbolic", t!("New Folder…"));
        let uploads = gio::Menu::new();
        uploads.append(Some(t!("Upload Files…")), Some("files.upload-files"));
        uploads.append(Some(t!("Upload Folders…")), Some("files.upload-folders"));
        let upload = gtk::MenuButton::builder().icon_name("document-send-symbolic").tooltip_text(t!("Upload")).menu_model(&uploads).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let open_folder = gtk::Button::builder().label(t!("Open in Files")).tooltip_text(t!("Open in Files")).valign(gtk::Align::Center).margin_start(4).build();
        let top = gtk::Box::builder().spacing(2).build();
        top.append(&gtk::ScrolledWindow::builder().child(&crumbs).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).hexpand(true).css_classes(["inner-scroll"]).build());
        top.append(&search_toggle);
        top.append(&sort_menu);
        top.append(&new_folder);
        top.append(&upload);
        top.append(&open_folder);

        let search = gtk::SearchEntry::builder().placeholder_text(t!("Search This Folder")).hexpand(true).build();
        let search_bar = gtk::Box::builder().visible(false).build();
        search_bar.append(&search);

        let clear = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text(t!("Clear Selection")).valign(gtk::Align::Center).css_classes(["flat", "circular"]).build();
        let picked = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading", "numeric"]).build();
        let action = |label: &str| gtk::Button::builder().label(label).tooltip_text(label).valign(gtk::Align::Center).css_classes(["flat"]).build();
        let (copy, rename, delete) = (action(t!("Copy to Computer")), action(t!("Rename…")), action(t!("Delete…")));
        delete.add_css_class("error");
        let picked_bar = gtk::Box::builder().spacing(4).visible(false).css_classes(["files-picked"]).build();
        picked_bar.append(&clear);
        picked_bar.append(&picked);
        picked_bar.append(&copy);
        picked_bar.append(&rename);
        picked_bar.append(&delete);

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let query: Rc<RefCell<String>> = Rc::default();
        let q = query.clone();
        let filter = gtk::CustomFilter::new(move |obj| {
            let q = q.borrow();
            q.is_empty() || item(obj).lower.contains(q.as_str())
        });
        let sort = Rc::new(Cell::new((SortBy::Name, false)));
        let s = sort.clone();
        let sorter = gtk::CustomSorter::new(move |a, b| {
            let (a, b, (by, descending)) = (item(a), item(b), s.get());
            let order = match by {
                SortBy::Name => a.key.cmp(&b.key),
                SortBy::Date => a.entry.modified_ms.cmp(&b.entry.modified_ms).then_with(|| a.key.cmp(&b.key)),
                SortBy::Size => a.entry.size.cmp(&b.entry.size).then_with(|| a.key.cmp(&b.key)),
            };
            // Folders first, whichever the order
            b.entry.dir.cmp(&a.entry.dir).then(if descending { order.reverse() } else { order }).into()
        });
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let sorted = gtk::SortListModel::new(Some(filtered), Some(sorter.clone()));
        let shown = gtk::SliceListModel::new(Some(sorted.clone()), 0, ROWS);
        let selection = gtk::MultiSelection::new(Some(shown.clone()));

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let icon = gtk::Image::builder().pixel_size(22).build();
            let name = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["inbox-name"]).build();
            let detail = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label", "numeric"]).build();
            let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
            text.append(&name);
            text.append(&detail);
            let next = gtk::Image::builder().icon_name("go-next-symbolic").css_classes(["dim-label"]).build();
            let b = gtk::Box::builder().spacing(14).margin_top(9).margin_bottom(9).margin_start(10).margin_end(10).build();
            b.append(&icon);
            b.append(&text);
            b.append(&next);
            item.set_child(Some(&b));
        });
        factory.connect_bind(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().expect("list item");
            let (Some(obj), Some(row)) = (list_item.item(), list_item.child()) else { return };
            let it = item(&obj);
            let e = &it.entry;
            let icon = row.first_child().and_downcast::<gtk::Image>().expect("icon");
            let text = icon.next_sibling().expect("text");
            let next = text.next_sibling().expect("next");
            let name = text.first_child().and_downcast::<gtk::Label>().expect("name");
            let detail = name.next_sibling().and_downcast::<gtk::Label>().expect("detail");
            icon.set_icon_name(Some(if e.dir { "folder-symbolic" } else { file_icon(&e.name) }));
            name.set_label(&e.name);
            let when = (e.modified_ms > 0).then(|| super::page::day_label(e.modified_ms));
            let what = if e.dir { None } else { Some(fmt::size(e.size)) };
            detail.set_label(&[what, when].into_iter().flatten().collect::<Vec<_>>().join(" · "));
            detail.set_visible(!detail.label().is_empty());
            next.set_visible(e.dir);
        });
        // In the page, not a box of its own: it grows down and the page scrolls
        let list = gtk::ListView::builder().model(&selection).factory(&factory).enable_rubberband(true).css_classes(["files-list"]).build();
        let status = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(28).margin_bottom(28).build();
        let loading = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).halign(gtk::Align::Center).margin_top(28).margin_bottom(28).build();
        loading.append(&super::spinner());
        loading.append(&gtk::Label::builder().label(t!("Loading…")).css_classes(["dim-label"]).build());
        let stack = gtk::Stack::builder().vhomogeneous(false).css_classes(["files-area"]).build();
        stack.add_named(&list, Some("list"));
        stack.add_named(&status, Some("status"));
        stack.add_named(&loading, Some("loading"));
        let copies = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).visible(false).css_classes(["files-copies"]).build();

        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        root.append(&top);
        root.append(&search_bar);
        root.append(&picked_bar);
        root.append(&stack);
        root.append(&copies);
        let labelled = vec![
            (copy.clone(), "folder-download-symbolic", t!("Copy to Computer")),
            (rename.clone(), "document-edit-symbolic", t!("Rename…")),
            (delete.clone(), "edit-delete-symbolic", t!("Delete…")),
            (open_folder.clone(), "folder-open-symbolic", t!("Open in Files")),
        ];
        Rc::new(Self {
            root,
            crumbs,
            store,
            filter,
            sorter,
            sorted,
            shown,
            selection,
            list,
            stack,
            status,
            search_toggle,
            search_bar,
            search,
            query,
            sort,
            sorts,
            orders,
            new_folder,
            open_folder,
            picked_bar,
            picked,
            copy,
            rename,
            delete,
            clear,
            labelled,
            copies,
            rows: RefCell::default(),
            loading: Cell::new(None),
            loads: Cell::new(0),
            state: RefCell::default(),
        })
    }

    /// On a narrow page the actions show only their icons.
    pub fn set_narrow(&self, narrow: bool) {
        for (button, icon, label) in &self.labelled {
            if narrow {
                button.set_icon_name(icon);
            } else {
                button.set_label(label);
            }
        }
    }

    /// The page reached the end of the rows: more of them.
    pub fn grow(&self) {
        if self.shown.size() < self.sorted.n_items() {
            self.shown.set_size(self.shown.size() + ROWS);
        }
    }

    /// Where `name` is in the list, with rows built up to it.
    fn reveal(&self, name: &str) -> Option<u32> {
        let at = (0..self.sorted.n_items()).find(|&i| self.sorted.item(i).is_some_and(|o| item(&o).entry.name == name))?;
        if at >= self.shown.size() {
            self.shown.set_size(at + ROWS);
        }
        Some(at)
    }

    fn status(&self, text: &str) {
        self.status.set_label(text);
        self.stack.set_visible_child_name("status");
    }

    /// The list, or why there's nothing in it.
    fn show_list(&self) {
        let query = self.query.borrow().clone();
        if self.store.n_items() == 0 {
            self.status(t!("This folder is empty."));
        } else if self.selection.n_items() == 0 {
            self.status(&t!("Nothing here matches “{query}”.", query));
        } else {
            self.stack.set_visible_child_name("list");
        }
    }

    /// What's picked, in the order shown.
    fn picked(&self) -> Vec<FsEntry> {
        let set = self.selection.selection();
        (0..set.size().min(u32::MAX as u64) as u32).filter_map(|i| self.selection.item(set.nth(i))).map(|obj| item(&obj).entry.clone()).collect()
    }

    fn picked_changed(&self) {
        let count = self.selection.selection().size();
        self.picked_bar.set_visible(count > 0);
        self.picked.set_label(&t!("{n} selected", n = count));
        self.rename.set_sensitive(count == 1);
    }

    fn set_query(&self, text: &str) {
        let text = text.trim().to_lowercase();
        if *self.query.borrow() == text {
            return;
        }
        *self.query.borrow_mut() = text;
        self.filter.changed(gtk::FilterChange::Different);
        // Not over a folder still loading, or why it couldn't
        if self.loading.get().is_none() && self.store.n_items() > 0 {
            self.show_list();
        }
    }

    /// The copies of this phone show; others wait until it's back.
    fn show_copies(&self, phone: Option<Fingerprint>) {
        let rows = self.rows.borrow();
        for row in rows.values() {
            row.root.set_visible(Some(row.phone) == phone);
        }
        self.copies.set_visible(rows.values().any(|r| Some(r.phone) == phone));
    }
}

/// Sort by name, date or size, either way: the menu, and its choices.
fn sort_menu() -> (gtk::MenuButton, Choices<SortBy>, Choices<bool>) {
    let menu = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(6).margin_bottom(6).margin_start(6).margin_end(6).build();
    menu.append(&gtk::Label::builder().label(t!("Sort")).xalign(0.0).margin_start(6).margin_top(4).margin_bottom(2).css_classes(["caption-heading", "dim-label"]).build());
    let (mut sorts, mut orders) = (Vec::new(), Vec::new());
    let mut first: Option<gtk::CheckButton> = None;
    for (label, by) in [(t!("Name"), SortBy::Name), (t!("Date"), SortBy::Date), (t!("Size"), SortBy::Size)] {
        let b = gtk::CheckButton::builder().label(label).active(by == SortBy::Name).build();
        b.set_group(first.as_ref());
        first.get_or_insert(b.clone());
        menu.append(&b);
        sorts.push((by, b));
    }
    menu.append(&gtk::Separator::builder().margin_top(6).margin_bottom(4).build());
    let mut first: Option<gtk::CheckButton> = None;
    for (label, descending) in [(t!("Ascending"), false), (t!("Descending"), true)] {
        let b = gtk::CheckButton::builder().label(label).active(!descending).build();
        b.set_group(first.as_ref());
        first.get_or_insert(b.clone());
        menu.append(&b);
        orders.push((descending, b));
    }
    let button = gtk::MenuButton::builder()
        .icon_name("view-sort-descending-symbolic")
        .tooltip_text(t!("Sort"))
        .popover(&gtk::Popover::builder().child(&menu).build())
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    (button, sorts, orders)
}

impl Window {
    pub(super) fn setup_files(self: &Rc<Self>) {
        let f = &self.files;
        let w = Rc::downgrade(self);
        f.list.connect_activate(move |_, position| {
            let Some(w) = w.upgrade() else { return };
            let Some(obj) = w.files.selection.item(position) else { return };
            let e = item(&obj).entry.clone();
            let path = format!("{}{}", w.files.state.borrow().path, e.name);
            if e.dir {
                w.files_open(format!("{path}/"));
            } else {
                w.files_launch(path);
            }
        });
        let w = Rc::downgrade(self);
        f.open_folder.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            let (path, phone) = {
                let state = w.files.state.borrow();
                (state.path.clone(), state.phone)
            };
            match phone {
                // Its top folder also goes in the Files sidebar.
                Some(phone) if path == "/" => w.browse_files(phone),
                _ => w.files_launch(path),
            }
        });

        let v = Rc::downgrade(f);
        f.selection.connect_selection_changed(move |_, _, _| {
            if let Some(v) = v.upgrade() {
                v.picked_changed();
            }
        });
        let v = Rc::downgrade(f);
        f.selection.connect_items_changed(move |_, _, _, _| {
            if let Some(v) = v.upgrade() {
                v.picked_changed();
            }
        });
        let v = Rc::downgrade(f);
        f.clear.connect_clicked(move |_| {
            if let Some(v) = v.upgrade() {
                v.selection.unselect_all();
            }
        });
        let w = Rc::downgrade(self);
        f.copy.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.files_copy();
            }
        });
        let w = Rc::downgrade(self);
        f.rename.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.files_rename();
            }
        });
        let w = Rc::downgrade(self);
        f.delete.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.files_delete();
            }
        });
        let w = Rc::downgrade(self);
        f.new_folder.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.files_new_folder();
            }
        });
        let actions = gio::SimpleActionGroup::new();
        for (name, folders) in [("upload-files", false), ("upload-folders", true)] {
            let action = gio::SimpleAction::new(name, None);
            let w = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    w.files_choose_upload(folders);
                }
            });
            actions.add_action(&action);
        }
        f.root.insert_action_group("files", Some(&actions));

        // Search: this folder, by name, as it's typed
        let v = Rc::downgrade(f);
        f.search_toggle.connect_toggled(move |b| {
            let Some(v) = v.upgrade() else { return };
            v.search_bar.set_visible(b.is_active());
            if b.is_active() {
                v.search.grab_focus();
            } else {
                v.search.set_text("");
                v.set_query("");
            }
        });
        let v = Rc::downgrade(f);
        f.search.connect_search_changed(move |e| {
            if let Some(v) = v.upgrade() {
                v.set_query(&e.text());
            }
        });
        let v = Rc::downgrade(f);
        f.search.connect_stop_search(move |_| {
            if let Some(v) = v.upgrade() {
                v.search_toggle.set_active(false);
                v.list.grab_focus();
            }
        });
        for (by, b) in &f.sorts {
            let (v, by) = (Rc::downgrade(f), *by);
            b.connect_toggled(move |b| {
                if let Some(v) = v.upgrade()
                    && b.is_active()
                {
                    v.sort.set((by, v.sort.get().1));
                    v.sorter.changed(gtk::SorterChange::Different);
                }
            });
        }
        for (descending, b) in &f.orders {
            let (v, descending) = (Rc::downgrade(f), *descending);
            b.connect_toggled(move |b| {
                if let Some(v) = v.upgrade()
                    && b.is_active()
                {
                    v.sort.set((v.sort.get().0, descending));
                    v.sorter.changed(gtk::SorterChange::Different);
                }
            });
        }

        // Keys, as in Files: Delete, F2 to rename, Escape to let go, Ctrl+F to search
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(w) = w.upgrade() else { return glib::Propagation::Proceed };
            let picked = w.files.selection.selection().size();
            match key {
                gdk::Key::Delete | gdk::Key::KP_Delete if picked > 0 => w.files_delete(),
                gdk::Key::F2 if picked == 1 => w.files_rename(),
                gdk::Key::Escape if picked > 0 => {
                    w.files.selection.unselect_all();
                }
                gdk::Key::f if mods.contains(gdk::ModifierType::CONTROL_MASK) => w.files.search_toggle.set_active(true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        f.list.add_controller(keys);

        // Files dropped on the list go into the folder shown
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let v = Rc::downgrade(f);
        target.connect_enter(move |_, _, _| {
            if let Some(v) = v.upgrade() {
                v.stack.add_css_class("drop-active");
            }
            gdk::DragAction::COPY
        });
        let v = Rc::downgrade(f);
        target.connect_leave(move |_| {
            if let Some(v) = v.upgrade() {
                v.stack.remove_css_class("drop-active");
            }
        });
        let w = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let Some(w) = w.upgrade() else { return false };
            w.files.stack.remove_css_class("drop-active");
            let paths = super::file_list_paths(value);
            if paths.is_empty() {
                return false;
            }
            w.files_upload(paths);
            true
        });
        f.stack.add_controller(target);
    }

    /// The Files tab shows: the phone's top folder the first time, else where it was.
    pub(super) fn files_shown(self: &Rc<Self>, phone: Fingerprint) {
        let same = self.files.state.borrow().phone == Some(phone);
        if !same {
            *self.files.state.borrow_mut() = FilesState { phone: Some(phone), path: "/".into(), place: None };
            self.files.store.remove_all();
        }
        self.files.show_copies(Some(phone));
        let path = self.files.state.borrow().path.clone();
        self.files_load(path, None);
    }

    /// Go to a folder: a fresh look, nothing picked, no search.
    fn files_open(self: &Rc<Self>, path: String) {
        self.files.search_toggle.set_active(false);
        self.files_load(path, None);
    }

    /// The folder shown again (after a change in it), picking `pick` once it's there.
    fn files_reload(self: &Rc<Self>, pick: Option<String>) {
        let path = self.files.state.borrow().path.clone();
        self.files_load(path, pick);
    }

    fn files_load(self: &Rc<Self>, path: String, pick: Option<String>) {
        let (Some(c), Some(phone)) = (self.client(), self.files.state.borrow().phone) else { return };
        let moved = std::mem::replace(&mut self.files.state.borrow_mut().path, path.clone()) != path;
        self.files_crumbs(phone, &path);
        let load = self.files.loads.get() + 1;
        self.files.loads.set(load);
        self.files.loading.set(Some(load));
        // Only a folder that takes a while says it's loading
        if self.files.store.n_items() == 0 || moved {
            let w = Rc::downgrade(self);
            let wait = if self.files.store.n_items() == 0 { 0 } else { 300 };
            glib::timeout_add_local_once(Duration::from_millis(wait), move || {
                if let Some(w) = w.upgrade()
                    && w.files.loading.get() == Some(load)
                {
                    w.files.stack.set_visible_child_name("loading");
                }
            });
        }
        let p = path.clone();
        let call = self.bridge.call(&c, move |c| async move { c.fs_list(phone.to_string(), p).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            let result = call.await;
            if w.files.loading.get() != Some(load) || w.files.state.borrow().phone != Some(phone) {
                return;
            }
            w.files.loading.set(None);
            match result {
                Ok(entries) => {
                    let items: Vec<glib::BoxedAnyObject> = entries.into_iter().filter(|e| !e.name.starts_with('.')).map(new_item).collect();
                    if moved {
                        w.files.shown.set_size(ROWS);
                    }
                    w.files.store.splice(0, w.files.store.n_items(), &items);
                    w.files.show_list();
                    match pick.and_then(|name| w.files.reveal(&name)) {
                        Some(i) => w.files.list.scroll_to(i, gtk::ListScrollFlags::SELECT | gtk::ListScrollFlags::FOCUS, None),
                        None if moved => w.page.scroll_to_tabs(),
                        None => {}
                    }
                }
                Err(e) => {
                    w.files.store.remove_all();
                    w.files.status(&fmt::capitalize(&e));
                }
            }
        });
    }

    /// The phone's name, then each folder down to this one, each a button back to it.
    fn files_crumbs(self: &Rc<Self>, phone: Fingerprint, path: &str) {
        let crumbs = &self.files.crumbs;
        while let Some(c) = crumbs.first_child() {
            crumbs.remove(&c);
        }
        let name = self.device(&phone).map(|d| d.device.name).unwrap_or_default();
        let mut at = String::from("/");
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for (i, label) in std::iter::once(name.as_str()).chain(parts.iter().copied()).enumerate() {
            if i > 0 {
                at.push_str(label);
                at.push('/');
                crumbs.append(&gtk::Image::builder().icon_name("go-next-symbolic").pixel_size(12).css_classes(["dim-label"]).build());
            }
            let last = i == parts.len();
            let b = gtk::Button::builder().label(label).css_classes(if last { vec!["flat", "crumb", "current"] } else { vec!["flat", "crumb"] }).build();
            let (w, target) = (Rc::downgrade(self), at.clone());
            b.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.files_open(target.clone());
                }
            });
            crumbs.append(&b);
        }
    }

    /// Open a file, or a folder in the file manager, through the phone's bridge.
    fn files_launch(self: &Rc<Self>, path: String) {
        let (Some(c), Some(phone)) = (self.client(), self.files.state.borrow().phone) else { return };
        let known = self.files.state.borrow().place.clone();
        let call = self.bridge.call(&c, move |c| async move {
            match known {
                Some(place) => Ok(place),
                None => c.fs_place(phone.to_string()).await,
            }
        });
        let w = self.clone();
        glib::spawn_future_local(async move {
            match call.await {
                Ok(place) => {
                    w.files.state.borrow_mut().place = Some(place.clone());
                    let escaped: Vec<String> = path.split('/').map(|p| glib::Uri::escape_string(p, None, false).to_string()).collect();
                    let file = gio::File::for_uri(&format!("{}{}", place.trim_end_matches('/'), escaped.join("/")));
                    let w2 = w.clone();
                    gtk::FileLauncher::new(Some(&file)).launch(Some(&w.window), gio::Cancellable::NONE, move |r| {
                        if let Err(e) = r
                            && !e.matches(gtk::DialogError::Dismissed)
                        {
                            w2.toast(&fmt::capitalize(&e.to_string()));
                        }
                    });
                }
                Err(e) => w.toast(&fmt::capitalize(&e)),
            }
        });
    }

    /// The phone and the folder shown, with the path of each picked item.
    fn files_here(&self) -> Option<(Fingerprint, String, Vec<FsEntry>)> {
        let state = self.files.state.borrow();
        Some((state.phone?, state.path.clone(), self.files.picked()))
    }

    fn files_copy(self: &Rc<Self>) {
        let (Some(c), Some((phone, dir, picked))) = (self.client(), self.files_here()) else { return };
        if picked.is_empty() {
            return;
        }
        let paths = picked.iter().map(|e| format!("{dir}{}", e.name)).collect();
        let call = self.bridge.call(&c, move |c| async move { c.fs_copy(phone.to_string(), paths).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(phone));
            }
        });
    }

    fn files_new_folder(self: &Rc<Self>) {
        let w = self.clone();
        self.files_ask_name(t!("New Folder"), t!("Create"), "", move |name| {
            let (Some(c), Some((phone, dir, _))) = (w.client(), w.files_here()) else { return };
            let path = format!("{dir}{name}");
            let call = w.bridge.call(&c, move |c| async move { c.fs_mkdir(phone.to_string(), path).await });
            let w = w.clone();
            glib::spawn_future_local(async move {
                match call.await {
                    Ok(()) => w.files_reload(Some(name)),
                    Err(e) => w.toast_error(&e, Some(phone)),
                }
            });
        });
    }

    fn files_rename(self: &Rc<Self>) {
        let Some((_, _, picked)) = self.files_here() else { return };
        let [entry] = picked.as_slice() else { return };
        let (w, old) = (self.clone(), entry.name.clone());
        self.files_ask_name(&t!("Rename “{name}”", name = entry.name), t!("Rename"), &entry.name, move |to| {
            let (Some(c), Some((phone, dir, _))) = (w.client(), w.files_here()) else { return };
            let (path, name) = (format!("{dir}{old}"), to.clone());
            let call = w.bridge.call(&c, move |c| async move { c.fs_rename(phone.to_string(), path, to).await });
            let w = w.clone();
            glib::spawn_future_local(async move {
                match call.await {
                    Ok(_) => w.files_reload(Some(name)),
                    Err(e) => w.toast_error(&e, Some(phone)),
                }
            });
        });
    }

    /// Ask for a name for something in the folder shown: none taken there, no "/".
    fn files_ask_name(self: &Rc<Self>, heading: &str, confirm: &str, initial: &str, done: impl Fn(String) + 'static) {
        let dialog = adw::AlertDialog::new(Some(heading), None);
        let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
        let hint = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["caption", "error"]).visible(false).build();
        let fields = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        fields.append(&entry);
        fields.append(&hint);
        dialog.set_extra_child(Some(&fields));
        dialog.add_responses(&[("cancel", t!("Cancel")), ("ok", confirm)]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let store = &self.files.store;
        let taken: HashSet<String> = (0..store.n_items()).filter_map(|i| store.item(i)).map(|o| item(&o).lower.clone()).collect();
        let check = {
            let (dialog, entry, hint, initial) = (dialog.clone(), entry.clone(), hint.clone(), initial.to_string());
            move || {
                let name = entry.text();
                let renamed = name.to_lowercase() != initial.to_lowercase();
                let problem = if name.contains('/') {
                    Some(t!("A name can't have “/” in it."))
                } else if renamed && taken.contains(&name.to_lowercase()) {
                    Some(t!("Something with this name is already here."))
                } else {
                    None
                };
                hint.set_label(problem.unwrap_or_default());
                hint.set_visible(problem.is_some());
                let empty = name.trim().is_empty() || name.as_str() == "." || name.as_str() == "..";
                dialog.set_response_enabled("ok", problem.is_none() && !empty && name.as_str() != initial);
            }
        };
        check();
        entry.connect_changed(move |_| check());
        let e = entry.clone();
        dialog.connect_response(Some("ok"), move |_, _| done(e.text().to_string()));
        dialog.present(Some(&self.window));
        // A file's name is picked up to its extension, ready to type over
        let stem = match initial.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() && self.files.picked().first().is_some_and(|e| !e.dir) => stem.chars().count(),
            _ => initial.chars().count(),
        };
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, stem as i32);
        });
    }

    fn files_delete(self: &Rc<Self>) {
        let Some((phone, dir, picked)) = self.files_here() else { return };
        let name = self.device(&phone).map(|d| d.device.name).unwrap_or_default();
        let (heading, body) = match picked.as_slice() {
            [] => return,
            [one] => (t!("Delete “{name}”?", name = one.name), t!("It's deleted from {phone} for good.", phone = name)),
            many => (t!("Delete {n} Items?", n = many.len()), t!("They're deleted from {phone} for good.", phone = name)),
        };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_responses(&[("cancel", t!("Cancel")), ("delete", t!("Delete"))]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let w = self.clone();
        dialog.connect_response(Some("delete"), move |_, _| {
            let Some(c) = w.client() else { return };
            let paths = picked.iter().map(|e| format!("{dir}{}", e.name)).collect();
            let call = w.bridge.call(&c, move |c| async move { c.fs_delete(phone.to_string(), paths).await });
            let w = w.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = call.await {
                    w.toast_error(&e, Some(phone));
                }
                w.files_reload(None);
            });
        });
        dialog.present(Some(&self.window));
    }

    fn files_choose_upload(self: &Rc<Self>, folders: bool) {
        let Some(phone) = self.files.state.borrow().phone else { return };
        let name = self.device(&phone).map(|d| d.device.name).unwrap_or_default();
        let dialog = gtk::FileDialog::builder().title(t!("Upload to {name}", name)).accept_label(t!("Upload")).modal(true).build();
        let w = self.clone();
        let chosen = move |res: Result<gio::ListModel, glib::Error>| {
            let Ok(list) = res else { return };
            let paths: Vec<PathBuf> = (0..list.n_items()).filter_map(|i| list.item(i).and_downcast::<gio::File>()).filter_map(|f| f.path()).collect();
            if !paths.is_empty() {
                w.files_upload(paths);
            }
        };
        if folders {
            dialog.select_multiple_folders(Some(&self.window), gio::Cancellable::NONE, chosen);
        } else {
            dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, chosen);
        }
    }

    /// Files and folders from here into the folder shown.
    fn files_upload(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let (Some(c), Some((phone, dir, _))) = (self.client(), self.files_here()) else { return };
        let call = self.bridge.call(&c, move |c| async move { c.fs_upload(phone.to_string(), dir, paths).await });
        let w = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = call.await {
                w.toast_error(&e, Some(phone));
            }
        });
    }

    /// A copy to or from a phone went on, or ended.
    pub(super) fn files_progress(self: &Rc<Self>, p: FsProgress) {
        let shown = self.files.state.borrow().phone;
        if p.finished {
            if let Some(row) = self.files.rows.borrow_mut().remove(&p.op) {
                self.files.copies.remove(&row.root);
            }
            self.files.show_copies(shown);
            self.files_finished(&p);
            return;
        }
        let mut rows = self.files.rows.borrow_mut();
        let row = rows.entry(p.op).or_insert_with(|| self.files_copy_row(&p));
        let percent = (p.total > 0).then(|| p.done as f64 / p.total as f64);
        row.title.set_label(&match p.dir {
            Some(_) => t!("Uploading {name}", name = p.name),
            None => t!("Copying {name}", name = p.name),
        });
        let mut detail = t!("{done} of {total}", done = fmt::size(p.done), total = fmt::size(p.total));
        if let Some(f) = percent {
            detail.push_str(&format!(" · {}%", (f * 100.0) as u32));
        }
        row.detail.set_label(&detail);
        match percent {
            Some(f) => row.bar.set_fraction(f),
            None => row.bar.pulse(),
        }
        drop(rows);
        self.files.show_copies(shown);
    }

    fn files_copy_row(self: &Rc<Self>, p: &FsProgress) -> CopyRow {
        let icon = gtk::Image::builder().icon_name(if p.dir.is_some() { "document-send-symbolic" } else { "folder-download-symbolic" }).valign(gtk::Align::Start).css_classes(["dim-label"]).build();
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["inbox-name"]).build();
        let detail = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label", "numeric"]).build();
        let bar = gtk::ProgressBar::builder().margin_top(4).css_classes(["transfer-progress"]).build();
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        text.append(&title);
        text.append(&detail);
        text.append(&bar);
        let cancel = gtk::Button::builder().label(t!("Cancel")).valign(gtk::Align::Center).css_classes(["flat", "small"]).build();
        let (w, op) = (Rc::downgrade(self), p.op);
        cancel.connect_clicked(move |b| {
            let Some(w) = w.upgrade() else { return };
            let Some(c) = w.client() else { return };
            b.set_sensitive(false);
            let call = w.bridge.call(&c, move |c| async move { c.fs_cancel(op).await });
            glib::spawn_future_local(async move {
                let _ = call.await;
            });
        });
        let root = gtk::Box::builder().spacing(12).build();
        root.append(&icon);
        root.append(&text);
        root.append(&cancel);
        self.files.copies.append(&root);
        CopyRow { phone: p.phone, root, title, detail, bar }
    }

    /// A copy ended: where it went, or why it didn't; an upload's folder shows what came.
    fn files_finished(self: &Rc<Self>, p: &FsProgress) {
        match (&p.error, &p.dir) {
            (Some(e), _) if e == "cancelled" => {}
            (Some(e), None) => self.toast(&t!("Couldn't copy {name}: {error}", name = p.name, error = e)),
            (Some(e), Some(_)) => self.toast(&t!("Couldn't upload {name}: {error}", name = p.name, error = e)),
            (None, None) => {
                let Some(first) = p.saved.first().cloned() else { return };
                // Saved in <download folder>/<phone>/
                let folder = first.parent().and_then(Path::parent).and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let toast = adw::Toast::builder().title(glib::markup_escape_text(&t!("Copied to {folder}", folder))).button_label(t!("Show in Folder")).timeout(5).build();
                let window = self.window.clone();
                toast.connect_button_clicked(move |_| {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(&first))).open_containing_folder(Some(&window), gio::Cancellable::NONE, |_| {});
                });
                self.toasts.add_toast(toast);
            }
            (None, Some(_)) => {}
        }
        let here = {
            let state = self.files.state.borrow();
            state.phone == Some(p.phone) && p.dir.as_deref().is_some_and(|d| d.trim_end_matches('/') == state.path.trim_end_matches('/'))
        };
        if here {
            self.files_reload(None);
        }
    }
}

fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "dng" => "image-x-generic-symbolic",
        "mp4" | "mkv" | "webm" | "mov" | "3gp" => "video-x-generic-symbolic",
        "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" | "aac" => "audio-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}
