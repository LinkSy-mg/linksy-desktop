//! The deck editor: the widget library on the left, the phone with its grid in the middle,
//! the inspector on the right, and the phone's layouts, saved to the service in one piece.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::OnceLock;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib, pango};
use linksy_i18n::t;
use linksy_ipc::DeckPrefs;
use linksy_ipc::linksy_core::deck_layout::{MAX_LABEL, MAX_LAYOUTS, MAX_PAGES};
use linksy_ipc::linksy_core::{DeckFace, DeckKind, DeckLayout, DeckLayouts, DeckTheme, DeckTile, Fingerprint};

use super::art::Art;
use super::face::{self, Look};
use super::fit::Fit;
use super::grid::{Event, Grid};
use super::model::{self, Item, Metrics, Theme};
use super::{group_title, style};
use crate::fmt;
use crate::window::Window;

/// The phone, as the preview lays it out (then scales it).
const PORTRAIT: (i32, i32) = (300, 640);
const LANDSCAPE: (i32, i32) = (640, 300);

/// What the service says while it doesn't keep layouts.
const NOT_KEPT: &str = "deck layouts aren't kept here yet";

struct St {
    layouts: DeckLayouts,
    /// As the service has them, to tell what changed.
    saved: DeckLayouts,
    /// The service keeps this phone's layouts.
    kept: bool,
    saving: bool,
    layout: usize,
    landscape: bool,
    page: usize,
    picked: Option<String>,
    prefs: DeckPrefs,
    library: Vec<(String, Vec<Item>)>,
    query: String,
    flash: Option<glib::SourceId>,
    tick: Option<glib::SourceId>,
}

impl St {
    fn current(&self) -> &DeckLayout {
        &self.layouts.layouts[self.layout]
    }

    fn face(&self) -> &DeckFace {
        let l = self.current();
        if self.landscape { &l.landscape } else { &l.portrait }
    }

    fn face_mut(&mut self) -> &mut DeckFace {
        let landscape = self.landscape;
        let l = &mut self.layouts.layouts[self.layout];
        if landscape { &mut l.landscape } else { &mut l.portrait }
    }

    fn tiles(&self) -> &[DeckTile] {
        self.face().pages.get(self.page).map(Vec::as_slice).unwrap_or(&[])
    }

    fn tile(&self, id: &str) -> Option<&DeckTile> {
        self.tiles().iter().find(|t| t.id == id)
    }

    fn tile_mut(&mut self, id: &str) -> Option<&mut DeckTile> {
        let page = self.page;
        self.face_mut().pages.get_mut(page)?.iter_mut().find(|t| t.id == id)
    }

    fn theme(&self) -> Theme {
        Theme::from_id(&self.current().theme.id)
    }

    fn offered(&self, action: &str) -> bool {
        self.prefs.actions.iter().any(|o| o.action.id == action)
    }

    /// A tile's name: its widget's in the library, else its action's, else its kind.
    fn name_of(&self, t: &DeckTile) -> String {
        if let Some(item) = model::item_for(&self.library, t) {
            return item.name.clone();
        }
        let action = t.action.as_deref().and_then(|a| self.prefs.actions.iter().find(|o| o.action.id == a));
        match (action, &t.action) {
            (Some(o), _) => o.action.name.clone(),
            (None, Some(a)) => a.clone(),
            (None, None) => t.kind.clone(),
        }
    }

    /// The actions the saved layouts run: a tile kept from before still saves where this
    /// computer stopped offering its action.
    fn saved_runs(&self) -> std::collections::HashSet<String> {
        self.saved.layouts.iter().flat_map(|l| [&l.portrait, &l.landscape]).flat_map(|f| f.pages.iter().flatten()).filter_map(|t| t.action.clone()).collect()
    }

    fn dirty(&self) -> bool {
        self.layouts != self.saved
    }

    fn new_id(&self) -> String {
        let taken = model::ids(&self.layouts);
        model::new_id(|id| taken.contains(id))
    }

    /// The grid's cells as the phone shows them.
    fn metrics(&self) -> Metrics {
        let (w, h) = if self.landscape { (598.0, 222.0) } else { (258.0, 524.0) };
        Metrics::fit(w, h, self.face().grid)
    }

    /// Keeps the page and the picked tile to what there is.
    fn settle(&mut self) {
        self.layout = self.layout.min(self.layouts.layouts.len().saturating_sub(1));
        self.page = self.page.min(self.face().pages.len().saturating_sub(1));
        if self.picked.as_ref().is_some_and(|id| self.tile(id).is_none()) {
            self.picked = None;
        }
    }
}

struct Ui {
    names: gtk::StringList,
    layouts: gtk::DropDown,
    portrait: gtk::ToggleButton,
    landscape: gtk::ToggleButton,
    chip: gtk::Label,
    save: gtk::Button,
    split: adw::OverlaySplitView,
    search: gtk::SearchEntry,
    groups: gtk::Box,
    themes: Vec<(Theme, gtk::ToggleButton)>,
    info: gtk::Label,
    note: gtk::Label,
    fit: Fit,
    phone: gtk::Box,
    screen: gtk::Box,
    status: gtk::Box,
    time: gtk::Label,
    deck_name: gtk::Label,
    page_info: gtk::Label,
    grid: Grid,
    dots: gtk::Box,
    inspector: gtk::Box,
    new_layout: gtk::Button,
    new_icon: gtk::Button,
    battery: gtk::Label,
}

pub struct Editor {
    pub root: adw::BreakpointBin,
    win: Weak<Window>,
    phone: Fingerprint,
    device: String,
    battery: Option<u8>,
    st: RefCell<St>,
    ui: Ui,
    /// Set while the editor itself sets its controls, so their handlers stay quiet.
    quiet: Cell<bool>,
}

/// Development aids for screenshots, read once: LINKSY_SCREENSHOT_DECK_THEME=<id> shows a
/// theme, LINKSY_SCREENSHOT_DECK_PICK=<n> picks the nth tile (1 is the first) and
/// LINKSY_SCREENSHOT_DECK_LANDSCAPE=1 shows landscape, reflowed from portrait if empty;
/// LINKSY_SCREENSHOT_DECK_DRAG=move,80,60 (or resize,…) shows the picked tile mid-drag;
/// LINKSY_SCREENSHOT_DECK_ON=1 shows the toggles on.
struct Aids {
    theme: Option<String>,
    pick: Option<usize>,
    landscape: bool,
    drag: Option<(bool, f64, f64)>,
    on: bool,
}

fn aids() -> &'static Aids {
    static AIDS: OnceLock<Aids> = OnceLock::new();
    AIDS.get_or_init(|| {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let drag = var("LINKSY_SCREENSHOT_DECK_DRAG").and_then(|d| {
            let mut parts = d.split(',');
            let resize = parts.next()? == "resize";
            Some((resize, parts.next()?.trim().parse().ok()?, parts.next()?.trim().parse().ok()?))
        });
        Aids { theme: var("LINKSY_SCREENSHOT_DECK_THEME"), pick: var("LINKSY_SCREENSHOT_DECK_PICK").and_then(|n| n.parse().ok()), landscape: var("LINKSY_SCREENSHOT_DECK_LANDSCAPE").is_some(), drag, on: var("LINKSY_SCREENSHOT_DECK_ON").is_some() }
    })
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).wrap_mode(pango::WrapMode::WordChar).css_classes(classes.to_vec()).build()
}

fn field(text: &str) -> gtk::Label {
    label(text, &["deck-field"])
}

fn column(spacing: i32) -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(spacing).build()
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn now() -> glib::DateTime {
    glib::DateTime::now_local().unwrap_or_else(|_| glib::DateTime::from_unix_utc(0).expect("epoch"))
}

impl Editor {
    pub fn new(win: &Rc<Window>, phone: Fingerprint, device: String, battery: Option<u8>, prefs: DeckPrefs, layouts: Result<DeckLayouts, String>) -> Rc<Self> {
        let (kept, saved, mut working) = match layouts {
            Ok(l) if !l.layouts.is_empty() => (true, l.clone(), l),
            Ok(l) => (true, l, model::starter(&prefs, t!("Main"))),
            Err(e) => {
                if e != NOT_KEPT {
                    win.toast(&fmt::capitalize(&e));
                }
                let starter = model::starter(&prefs, t!("Main"));
                (false, starter.clone(), starter)
            }
        };
        let layout = working.layouts.iter().position(|l| l.id == working.active).unwrap_or(0);
        let aids = aids();
        if let Some(theme) = &aids.theme {
            working.layouts[layout].theme.id = Theme::from_id(theme).id().into();
        }
        let library = model::library(&prefs);
        let st = St { layouts: working, saved, kept, saving: false, layout, landscape: false, page: 0, picked: None, prefs, library, query: String::new(), flash: None, tick: None };
        let (root, ui) = build_ui();
        let editor = Rc::new(Self { root, win: Rc::downgrade(win), phone, device, battery, st: RefCell::new(st), ui, quiet: Cell::new(false) });
        editor.connect();
        if aids.landscape {
            editor.set_orientation(true);
            if editor.st.borrow().face().pages.iter().all(Vec::is_empty) {
                editor.reflow();
            }
        }
        editor.render_library();
        editor.render(false);
        if let Some(n) = aids.pick {
            let id = editor.st.borrow().tiles().get(n.saturating_sub(1)).map(|t| t.id.clone());
            editor.pick(id);
        }
        if let Some((resize, dx, dy)) = aids.drag {
            let grid = editor.ui.grid.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                if let Some(grid) = grid.upgrade() {
                    grid.preview_drag(resize, dx, dy);
                }
            });
        }
        editor
    }

    pub fn dirty(&self) -> bool {
        self.st.borrow().dirty()
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// What this computer offers changed (a command added or removed, say).
    pub fn set_prefs(self: &Rc<Self>, prefs: DeckPrefs) {
        {
            let mut st = self.st.borrow_mut();
            st.library = model::library(&prefs);
            st.prefs = prefs;
        }
        self.render_library();
        self.render_grid(false);
        self.render_inspector();
    }

    fn connect(self: &Rc<Self>) {
        let ui = &self.ui;
        let e = Rc::downgrade(self);
        ui.grid.connect(move |event| {
            if let Some(e) = e.upgrade() {
                e.on_grid(event);
            }
        });
        let e = Rc::downgrade(self);
        ui.layouts.connect_selected_notify(move |d| {
            if let Some(e) = e.upgrade()
                && !e.quiet.get()
            {
                e.select_layout(d.selected() as usize);
            }
        });
        let e = Rc::downgrade(self);
        ui.portrait.connect_toggled(move |b| {
            if let Some(e) = e.upgrade()
                && !e.quiet.get()
                && b.is_active()
            {
                e.set_orientation(false);
            }
        });
        let e = Rc::downgrade(self);
        ui.landscape.connect_toggled(move |b| {
            if let Some(e) = e.upgrade()
                && !e.quiet.get()
                && b.is_active()
            {
                e.set_orientation(true);
            }
        });
        for (theme, button) in &ui.themes {
            let (e, theme) = (Rc::downgrade(self), *theme);
            button.connect_toggled(move |b| {
                if let Some(e) = e.upgrade()
                    && !e.quiet.get()
                    && b.is_active()
                {
                    e.set_theme(theme);
                }
            });
        }
        for button in [&ui.new_layout, &ui.new_icon] {
            let e = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(e) = e.upgrade() {
                    e.new_layout();
                }
            });
        }
        if let Some(level) = self.battery {
            ui.battery.set_label(&format!("{level}%"));
        }
        let e = Rc::downgrade(self);
        ui.save.connect_clicked(move |_| {
            if let Some(e) = e.upgrade() {
                e.save(None);
            }
        });
        let e = Rc::downgrade(self);
        ui.search.connect_search_changed(move |s| {
            if let Some(e) = e.upgrade() {
                e.st.borrow_mut().query = s.text().to_string();
                e.render_library();
            }
        });
        let e = Rc::downgrade(self);
        self.root.connect_map(move |_| {
            if let Some(e) = e.upgrade() {
                e.tick();
            }
        });
        let e = Rc::downgrade(self);
        self.root.connect_unmap(move |_| {
            if let Some(e) = e.upgrade()
                && let Some(id) = e.st.borrow_mut().tick.take()
            {
                id.remove();
            }
        });
    }

    /// The clock's tiles and the status bar's time, then again at the next minute.
    fn tick(self: &Rc<Self>) {
        if let Some(id) = self.st.borrow_mut().tick.take() {
            id.remove();
        }
        let now = now();
        self.ui.time.set_label(&now.format("%H:%M").map(String::from).unwrap_or_default());
        {
            let st = self.st.borrow();
            let build = |t: &DeckTile| self.build_tile(&st, t, &now);
            self.ui.grid.rebuild(|t| t.kind == "clock", &build);
        }
        let wait = Duration::from_millis(((60 - now.second()).max(1) as u64) * 1000 + 50);
        let e = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(wait, move || {
            if let Some(e) = e.upgrade() {
                e.st.borrow_mut().tick = None;
                e.tick();
            }
        });
        self.st.borrow_mut().tick = Some(id);
    }

    fn on_grid(self: &Rc<Self>, event: Event) {
        // A focus change while the grid is being set up comes back later
        if self.st.try_borrow_mut().is_err() {
            let e = self.clone();
            glib::idle_add_local_once(move || e.on_grid(event));
            return;
        }
        match event {
            Event::Pick(id) => self.pick(id),
            Event::Move(id, x, y) => self.move_tile(&id, x, y),
            Event::Resize(id, w, h) => self.resize(&id, w, h, None),
            Event::Nudge(id, dx, dy) => {
                let to = self.st.borrow().tile(&id).map(|t| (t.x as i16 + dx as i16, t.y as i16 + dy as i16));
                if let Some((x, y)) = to.filter(|&(x, y)| x >= 0 && y >= 0) {
                    self.move_tile(&id, x as u8, y as u8);
                }
            }
            Event::Delete(id) => self.delete(&id),
            Event::Drop(key, x, y) => self.add(&key, Some((x, y))),
        }
    }

    fn build_tile(&self, st: &St, t: &DeckTile, now: &glib::DateTime) -> gtk::Widget {
        let offered = |a: &str| st.offered(a);
        // The deck's live states don't reach the editor: toggles show off
        let on = |a: &str| aids().on && st.prefs.actions.iter().any(|o| o.action.id == a && o.action.kind == DeckKind::Toggle);
        let confirm = |a: &str| st.prefs.actions.iter().any(|o| o.action.id == a && o.action.confirm.is_some());
        let look = Look { theme: st.theme(), accent: &st.current().theme.accent, m: st.metrics(), now, offered: &offered, on: &on, confirm: &confirm };
        if t.label.is_empty() {
            // A tile without a label of its own shows its name, as on the phone
            return face::tile(&DeckTile { label: st.name_of(t), ..t.clone() }, &look);
        }
        face::tile(t, &look)
    }

    // ---- rendering ----

    fn render(self: &Rc<Self>, animate: bool) {
        self.render_grid(animate);
        self.render_chrome();
        self.render_inspector();
    }

    fn render_grid(&self, animate: bool) {
        let st = self.st.borrow();
        let now = now();
        let build = |t: &DeckTile| self.build_tile(&st, t, &now);
        self.ui.grid.sync(st.face().grid, st.tiles(), st.theme(), animate, &build);
        self.ui.grid.set_picked(st.picked.as_deref());
    }

    fn render_chrome(self: &Rc<Self>) {
        let st = self.st.borrow();
        let ui = &self.ui;
        self.quiet.set(true);
        let names: Vec<String> = st.layouts.layouts.iter().map(|l| l.name.clone()).collect();
        let shown: Vec<String> = (0..ui.names.n_items()).filter_map(|i| ui.names.string(i).map(String::from)).collect();
        if names != shown {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            ui.names.splice(0, ui.names.n_items(), &names);
        }
        ui.layouts.set_selected(st.layout as u32);
        ui.portrait.set_active(!st.landscape);
        ui.landscape.set_active(st.landscape);
        let theme = st.theme();
        for (t, b) in &ui.themes {
            b.set_active(*t == theme);
        }
        self.quiet.set(false);

        for t in Theme::ALL {
            ui.screen.remove_css_class(&format!("theme-{}", t.id()));
        }
        ui.screen.add_css_class(&format!("theme-{}", theme.id()));
        let (w, h) = if st.landscape { LANDSCAPE } else { PORTRAIT };
        ui.fit.set_size(w, h);
        if st.landscape {
            ui.phone.add_css_class("land");
        } else {
            ui.phone.remove_css_class("land");
        }
        ui.status.set_visible(!st.landscape);
        ui.deck_name.set_label(&st.current().name);
        let pages = st.face().pages.len();
        ui.page_info.set_label(&t!("Page {n} of {count}", n = st.page + 1, count = pages));
        let [cols, rows] = st.face().grid;
        let orientation = if st.landscape { t!("Landscape") } else { t!("Portrait") };
        ui.info.set_label(&t!("{orientation} · {cols} × {rows} grid · {device}", orientation, cols, rows, device = self.device.as_str()));
        style::load(&st.current().theme.accent);

        clear(&ui.dots);
        for i in 0..pages {
            let dot = gtk::Button::builder().tooltip_text(t!("Page {n}", n = i + 1)).valign(gtk::Align::Center).build();
            if i == st.page {
                dot.add_css_class("current");
            }
            let e = Rc::downgrade(self);
            dot.connect_clicked(move |_| {
                if let Some(e) = e.upgrade() {
                    e.go_page(i);
                }
            });
            ui.dots.append(&dot);
        }
        if pages < MAX_PAGES {
            let add = gtk::Button::builder().label("+").tooltip_text(t!("Add a Page")).css_classes(["add"]).valign(gtk::Align::Center).build();
            let e = Rc::downgrade(self);
            add.connect_clicked(move |_| {
                if let Some(e) = e.upgrade() {
                    e.add_page();
                }
            });
            ui.dots.append(&add);
        }
        if pages > 1 && st.tiles().is_empty() {
            let remove = gtk::Button::builder().label("−").tooltip_text(t!("Remove This Page")).css_classes(["remove"]).valign(gtk::Align::Center).build();
            let e = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(e) = e.upgrade() {
                    e.remove_page();
                }
            });
            ui.dots.append(&remove);
        }
        drop(st);
        self.render_state();
    }

    /// Saved, not saved, or not kept here at all.
    fn render_state(&self) {
        let st = self.st.borrow();
        let (text, done) = match (st.saving, st.kept, st.dirty()) {
            (true, _, _) => (t!("Saving…"), false),
            (_, false, _) => (t!("Not saved here yet"), false),
            (_, true, true) => (t!("Not saved"), false),
            (_, true, false) => (t!("Saved"), true),
        };
        self.ui.chip.set_label(text);
        // Narrow windows hide the chip; the button still says it
        self.ui.save.set_tooltip_text(Some(text));
        if done {
            self.ui.chip.remove_css_class("dirty");
        } else {
            self.ui.chip.add_css_class("dirty");
        }
        self.ui.save.set_sensitive(!st.saving);
        if st.flash.is_none() {
            self.ui.note.remove_css_class("flash");
            self.ui.note.set_label(t!("Drag to move · corner to resize · Delete removes"));
        }
    }

    /// A short message where the hints are.
    fn flash(self: &Rc<Self>, text: &str) {
        if let Some(id) = self.st.borrow_mut().flash.take() {
            id.remove();
        }
        self.ui.note.set_label(text);
        self.ui.note.add_css_class("flash");
        let e = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(2600), move || {
            if let Some(e) = e.upgrade() {
                e.st.borrow_mut().flash = None;
                e.render_state();
            }
        });
        self.st.borrow_mut().flash = Some(id);
    }

    fn render_library(self: &Rc<Self>) {
        let groups = &self.ui.groups;
        clear(groups);
        let st = self.st.borrow();
        let mut any = false;
        for (group, items) in &st.library {
            let shown: Vec<&Item> = items.iter().filter(|i| model::matches(i, &st.query)).collect();
            let commands = group == "Commands";
            if shown.is_empty() && !(commands && st.query.trim().is_empty()) {
                continue;
            }
            any = true;
            groups.append(&gtk::Label::builder().label(if commands { t!("Your Commands").to_string() } else { group_title(group) }).xalign(0.0).css_classes(["deck-group"]).build());
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["deck-items"]).build();
            for item in shown {
                list.append(&self.library_row(item, &st));
            }
            if commands {
                let add = gtk::Box::builder().spacing(10).css_classes(["deck-item"]).build();
                let ic = gtk::Box::builder().css_classes(["ic"]).build();
                ic.append(&gtk::Image::builder().icon_name("list-add-symbolic").hexpand(true).build());
                add.append(&ic);
                add.append(&gtk::Label::builder().label(t!("Add Command…")).css_classes(["name"]).xalign(0.0).build());
                list.append(&gtk::ListBoxRow::builder().child(&add).name("add-command").build());
            }
            let e = Rc::downgrade(self);
            list.connect_row_activated(move |_, row| {
                let Some(e) = e.upgrade() else { return };
                let key = row.widget_name();
                if key == "add-command" {
                    e.add_command();
                } else {
                    e.add(&key, None);
                    if e.ui.split.is_collapsed() {
                        e.ui.split.set_show_sidebar(false);
                    }
                }
            });
            groups.append(&list);
        }
        if !any {
            groups.append(&label(t!("No widget matches."), &["deck-hint"]));
        }
    }

    fn library_row(self: &Rc<Self>, item: &Item, st: &St) -> gtk::ListBoxRow {
        let key = key(item);
        let row = gtk::ListBoxRow::builder().name(key.as_str()).tooltip_text(t!("Add {name}", name = item.name.as_str())).build();
        let line = gtk::Box::builder().spacing(10).css_classes(["deck-item"]).build();
        let ic = gtk::Box::builder().css_classes(["ic"]).valign(gtk::Align::Center).build();
        let icon = Art::icon(item.icon(), 18, Theme::Stock);
        icon.set_hexpand(true);
        ic.append(&icon);
        line.append(&ic);
        let words = column(0);
        words.set_hexpand(true);
        words.set_valign(gtk::Align::Center);
        words.append(&gtk::Label::builder().label(&item.name).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["name"]).build());
        words.append(&gtk::Label::builder().label(&item.hint).xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["hint"]).build());
        line.append(&words);
        line.append(&gtk::Label::builder().label(if item.phone { t!("Phone") } else { t!("PC") }).valign(gtk::Align::Center).css_classes(["deck-tag"]).build());
        if let Some(id) = &item.command {
            let remove = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text(t!("Remove Command")).valign(gtk::Align::Center).css_classes(["flat", "circular"]).build();
            let (e, id) = (Rc::downgrade(self), id.clone());
            remove.connect_clicked(move |_| {
                if let Some(e) = e.upgrade() {
                    e.remove_command(&id);
                }
            });
            line.append(&remove);
        }
        row.set_child(Some(&line));

        // Dragged onto the phone, it lands under the pointer
        let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
        let k = key.clone();
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&k.to_value())));
        let (e, item_) = (Rc::downgrade(self), item.clone());
        drag.connect_drag_begin(move |_, drag| {
            let Some(e) = e.upgrade() else { return };
            let ghost = e.ghost(&item_);
            gtk::DragIcon::for_drag(drag).set_child(Some(&ghost));
            drag.set_hotspot(model::HOTSPOT as i32, model::HOTSPOT as i32);
            let size = model::sizes(item_.kind, e.st.borrow().face().grid).first().copied();
            e.ui.grid.set_incoming(size);
            if e.ui.split.is_collapsed() {
                e.ui.split.set_show_sidebar(false);
            }
        });
        let e = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(e) = e.upgrade() {
                e.ui.grid.set_incoming(None);
            }
        });
        row.add_controller(drag);
        let _ = st;
        row
    }

    /// What a library drag carries under the pointer: the tile it adds.
    fn ghost(&self, item: &Item) -> gtk::Widget {
        let st = self.st.borrow();
        let (w, h) = model::sizes(item.kind, st.face().grid).first().copied().unwrap_or((1, 1));
        let tile = DeckTile { id: String::new(), kind: item.kind.into(), w, h, label: item.name.clone(), color: item.color().into(), action: item.action.clone(), ..Default::default() };
        let widget = self.build_tile(&st, &tile, &now());
        let m = st.metrics();
        widget.set_size_request(m.span(w) as i32, m.span(h) as i32);
        widget.add_css_class("lifted");
        let scope = gtk::Box::builder().css_classes(["deck-ghost", &format!("theme-{}", st.theme().id())]).build();
        scope.append(&widget);
        scope.upcast()
    }

    fn render_inspector(self: &Rc<Self>) {
        let b = &self.ui.inspector;
        clear(b);
        let picked = self.st.borrow().picked.clone();
        match picked.and_then(|id| self.st.borrow().tile(&id).cloned()) {
            Some(t) => self.inspect_tile(&t),
            None => self.inspect_layout(),
        }
    }

    fn inspect_tile(self: &Rc<Self>, t: &DeckTile) {
        let b = &self.ui.inspector;
        let st = self.st.borrow();
        let item = model::item_for(&st.library, t).cloned();
        let name = st.name_of(t);
        b.append(&label(&name, &["title-4"]));

        b.append(&field(t!("Label")));
        let entry = gtk::Entry::builder().text(&t.label).placeholder_text(&name).max_length(MAX_LABEL as i32).build();
        let (e, id) = (Rc::downgrade(self), t.id.clone());
        entry.connect_changed(move |entry| {
            if let Some(e) = e.upgrade() {
                e.set_label(&id, &entry.text());
            }
        });
        b.append(&entry);

        b.append(&field(t!("Colour")));
        let color = if t.color.is_empty() { model::default_color(&t.kind, t.action.as_deref()).to_string() } else { t.color.clone() };
        b.append(&self.swatches(&model::COLORS, &color, {
            let (e, id) = (Rc::downgrade(self), t.id.clone());
            move |c| {
                if let Some(e) = e.upgrade() {
                    e.set_color(&id, c);
                }
            }
        }));
        if st.theme() != Theme::Pixel {
            b.append(&label(&t!("{theme} draws every tile alike; Pixel colours each one.", theme = st.theme().name()), &["deck-hint"]));
        }

        b.append(&field(t!("Size")));
        let sizes = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(6).row_spacing(6).max_children_per_line(4).build();
        let face = st.face();
        let without = DeckFace { grid: face.grid, pages: vec![st.tiles().iter().filter(|o| o.id != t.id).cloned().collect()] };
        for (w, h) in model::sizes(&t.kind, face.grid) {
            let current = (w, h) == (t.w, t.h);
            let room = current || without.fits(0, t.x, t.y, w, h, None) || without.free_spot(0, w, h).is_some();
            let button = gtk::ToggleButton::builder().label(format!("{w} × {h}")).active(current).sensitive(room).css_classes(["deck-size"]).build();
            let (e, id) = (Rc::downgrade(self), t.id.clone());
            button.connect_clicked(move |b| {
                if let Some(e) = e.upgrade() {
                    if current {
                        b.set_active(true);
                    } else {
                        e.resize(&id, w, h, Some(()));
                    }
                }
            });
            sizes.append(&button);
        }
        b.append(&sizes);

        if let Some(action) = &t.action {
            let slider = t.kind == "volume";
            let options: Vec<(String, String)> =
                st.prefs.actions.iter().filter(|o| (o.action.kind == DeckKind::Slider) == slider).map(|o| (o.action.id.clone(), o.action.name.clone())).collect();
            if options.len() > 1 {
                b.append(&field(t!("Runs")));
                let names: Vec<&str> = options.iter().map(|(_, n)| n.as_str()).collect();
                let pick = gtk::DropDown::from_strings(&names);
                if let Some(i) = options.iter().position(|(id, _)| id == action) {
                    pick.set_selected(i as u32);
                } else {
                    pick.set_selected(gtk::INVALID_LIST_POSITION);
                }
                let (e, id) = (Rc::downgrade(self), t.id.clone());
                pick.connect_selected_notify(move |d| {
                    if let (Some(e), Some((a, n))) = (e.upgrade(), options.get(d.selected() as usize)) {
                        e.set_action(&id, a, n);
                    }
                });
                b.append(&pick);
            }
        }
        let hint = item.as_ref().map(|i| i.hint.clone()).or_else(|| t.action.clone()).unwrap_or_default();
        let note = match (&item, &t.action) {
            (Some(i), _) if i.phone => t!("Runs on the phone · {hint}.", hint = hint.as_str()),
            (_, Some(_)) => t!("Runs on the computer · {hint}. Placing it allows it for {device}.", hint = hint.as_str(), device = self.device.as_str()),
            _ => t!("Runs on the computer · {hint}.", hint = hint.as_str()),
        };
        b.append(&label(&note, &["deck-where"]));
        if t.action.as_deref().is_some_and(|a| !st.offered(a)) {
            b.append(&label(t!("Not available on this computer"), &["deck-warn"]));
        }
        let question = t.action.as_deref().and_then(|a| st.prefs.actions.iter().find(|o| o.action.id == a)).and_then(|o| o.action.confirm.clone());
        if let Some(q) = question {
            b.append(&label(&format!("{}\n\u{201c}{q}\u{201d}", t!("Asks on the phone first")), &["deck-hint"]));
        }

        let actions = gtk::Box::builder().spacing(8).build();
        let duplicate = gtk::Button::with_label(t!("Duplicate"));
        let (e, id) = (Rc::downgrade(self), t.id.clone());
        duplicate.connect_clicked(move |_| {
            if let Some(e) = e.upgrade() {
                e.duplicate(&id);
            }
        });
        let delete = gtk::Button::builder().label(t!("Delete")).css_classes(["destructive-action"]).build();
        let (e, id) = (Rc::downgrade(self), t.id.clone());
        delete.connect_clicked(move |_| {
            if let Some(e) = e.upgrade() {
                e.delete(&id);
            }
        });
        actions.append(&duplicate);
        actions.append(&delete);
        b.append(&actions);
    }

    fn inspect_layout(self: &Rc<Self>) {
        let b = &self.ui.inspector;
        let st = self.st.borrow();
        let l = st.current();
        b.append(&label(&l.name, &["title-4"]));
        b.append(&field(t!("Layout name")));
        let entry = gtk::Entry::builder().text(&l.name).max_length(MAX_LABEL as i32).build();
        let e = Rc::downgrade(self);
        entry.connect_changed(move |entry| {
            if let Some(e) = e.upgrade() {
                e.rename(&entry.text());
            }
        });
        b.append(&entry);
        b.append(&field(t!("Theme")));
        b.append(&label(&t!("{theme}. Pick another above the phone. The phone draws it; it's saved with this layout.", theme = st.theme().name()), &["deck-where"]));
        b.append(&field(t!("Accent, for what's on")));
        b.append(&self.swatches(&model::ACCENTS, &l.theme.accent, {
            let e = Rc::downgrade(self);
            move |c| {
                if let Some(e) = e.upgrade() {
                    e.set_accent(c);
                }
            }
        }));
        let n = st.tiles().len();
        let count = match n {
            0 => t!("No tiles on this page yet. Add widgets from the left.").to_string(),
            1 => t!("1 tile on this page. Pick it to change it, or add widgets from the left.").to_string(),
            n => t!("{n} tiles on this page. Pick one to change it, or add widgets from the left.", n),
        };
        b.append(&label(&count, &["deck-where"]));

        let actions = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(8).row_spacing(8).max_children_per_line(2).build();
        let duplicate = gtk::Button::builder().label(t!("Duplicate Layout")).sensitive(st.layouts.layouts.len() < MAX_LAYOUTS).build();
        let e = Rc::downgrade(self);
        duplicate.connect_clicked(move |_| {
            if let Some(e) = e.upgrade() {
                e.duplicate_layout();
            }
        });
        let delete = gtk::Button::builder().label(t!("Delete Layout")).css_classes(["destructive-action"]).sensitive(st.layouts.layouts.len() > 1).build();
        let e = Rc::downgrade(self);
        delete.connect_clicked(move |_| {
            if let Some(e) = e.upgrade() {
                e.delete_layout();
            }
        });
        actions.append(&duplicate);
        actions.append(&delete);
        if st.landscape && l.landscape.pages.iter().all(Vec::is_empty) && l.portrait.pages.iter().any(|p| !p.is_empty()) {
            let reflow = gtk::Button::builder().label(t!("Start from Portrait")).css_classes(["suggested-action"]).build();
            let e = Rc::downgrade(self);
            reflow.connect_clicked(move |_| {
                if let Some(e) = e.upgrade() {
                    e.reflow();
                }
            });
            actions.append(&reflow);
        }
        b.append(&actions);

        b.append(&label(&t!("What {device} may do", device = self.device.as_str()), &["title-4"]));
        b.append(&label(t!("Placing a tile allows what it runs when you save."), &["deck-hint"]));
        let mut groups: Vec<(String, adw::PreferencesGroup)> = Vec::new();
        for option in &st.prefs.actions {
            let group = match groups.iter().find(|(g, _)| *g == option.action.group) {
                Some((_, g)) => g.clone(),
                None => {
                    let g = adw::PreferencesGroup::builder().title(group_title(&option.action.group)).build();
                    groups.push((option.action.group.clone(), g.clone()));
                    g
                }
            };
            let row = adw::SwitchRow::builder().title(glib::markup_escape_text(&option.action.name)).active(option.on).build();
            let (e, action, reverting) = (Rc::downgrade(self), option.action.id.clone(), Rc::new(Cell::new(false)));
            row.connect_active_notify(move |row| {
                if !reverting.get()
                    && let Some(e) = e.upgrade()
                {
                    e.allow(&action, row, reverting.clone());
                }
            });
            group.add(&row);
        }
        let order = |g: &str| model::GROUPS.iter().position(|o| *o == g).unwrap_or(model::GROUPS.len());
        groups.sort_by_key(|(g, _)| order(g));
        for (_, g) in groups {
            b.append(&g);
        }
    }

    fn swatches(&self, colors: &[&'static str], current: &str, pick: impl Fn(&'static str) + Clone + 'static) -> gtk::FlowBox {
        let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(7).row_spacing(7).max_children_per_line(7).homogeneous(true).halign(gtk::Align::Start).build();
        let mut first: Option<gtk::ToggleButton> = None;
        for &c in colors {
            let button = gtk::ToggleButton::builder().css_classes(["deck-swatch", &format!("c{}", &c[1..])]).tooltip_text(c).active(c.eq_ignore_ascii_case(current)).build();
            button.update_property(&[gtk::accessible::Property::Label(c)]);
            if let Some(f) = &first {
                button.set_group(Some(f));
            } else {
                first = Some(button.clone());
            }
            let pick = pick.clone();
            button.connect_toggled(move |b| {
                if b.is_active() {
                    pick(c);
                }
            });
            flow.append(&button);
        }
        flow
    }

    // ---- editing ----

    fn edited(self: &Rc<Self>, animate: bool) {
        self.st.borrow_mut().settle();
        self.render(animate);
    }

    fn pick(self: &Rc<Self>, id: Option<String>) {
        if self.st.borrow().picked == id {
            return;
        }
        self.st.borrow_mut().picked = id.clone();
        self.ui.grid.set_picked(id.as_deref());
        self.render_inspector();
    }

    /// Adds the library's `key` widget at `at` if it fits there, else where there's room.
    fn add(self: &Rc<Self>, key: &str, at: Option<(u8, u8)>) {
        let item = self.st.borrow().library.iter().flat_map(|(_, items)| items).find(|i| self::key(i) == key).cloned();
        let Some(item) = item else { return };
        let added = {
            let mut st = self.st.borrow_mut();
            let (face, page) = (st.face(), st.page);
            let spot = model::place(face, page, &model::sizes(item.kind, face.grid), at);
            spot.map(|(x, y, w, h)| {
                let id = st.new_id();
                let label: String = item.name.chars().take(MAX_LABEL).collect();
                let tile = DeckTile { id: id.clone(), kind: item.kind.into(), x, y, w, h, label, color: item.color().into(), action: item.action.clone(), ..Default::default() };
                st.face_mut().pages[page].push(tile);
                st.picked = Some(id.clone());
                id
            })
        };
        match added {
            Some(id) => {
                self.edited(true);
                if let Some(w) = self.ui.grid.tile_widget(&id) {
                    w.grab_focus();
                }
            }
            None => self.flash(t!("This page is full. Add a page with the + under the grid.")),
        }
    }

    fn move_tile(self: &Rc<Self>, id: &str, x: u8, y: u8) {
        let moved = {
            let mut st = self.st.borrow_mut();
            let fits = st.tile(id).is_some_and(|t| st.face().fits(st.page, x, y, t.w, t.h, Some(id)));
            if let (true, Some(t)) = (fits, st.tile_mut(id)) {
                (t.x, t.y) = (x, y);
            }
            fits
        };
        if moved {
            self.edited(true);
        }
    }

    /// A new size; where it doesn't fit in place (`moving` set), the first spot it does.
    fn resize(self: &Rc<Self>, id: &str, w: u8, h: u8, moving: Option<()>) {
        let done = {
            let mut st = self.st.borrow_mut();
            let Some(t) = st.tile(id).cloned() else { return };
            let spot = if st.face().fits(st.page, t.x, t.y, w, h, Some(id)) {
                Some((t.x, t.y))
            } else if moving.is_some() {
                let without = DeckFace { grid: st.face().grid, pages: vec![st.tiles().iter().filter(|o| o.id != id).cloned().collect()] };
                without.free_spot(0, w, h)
            } else {
                None
            };
            match (spot, st.tile_mut(id)) {
                (Some((x, y)), Some(t)) => {
                    (t.x, t.y, t.w, t.h) = (x, y, w, h);
                    true
                }
                _ => false,
            }
        };
        if done {
            self.edited(true);
        }
    }

    fn delete(self: &Rc<Self>, id: &str) {
        {
            let mut st = self.st.borrow_mut();
            let page = st.page;
            if let Some(tiles) = st.face_mut().pages.get_mut(page) {
                tiles.retain(|t| t.id != id);
            }
            st.picked = None;
        }
        self.edited(true);
    }

    fn duplicate(self: &Rc<Self>, id: &str) {
        let done = {
            let mut st = self.st.borrow_mut();
            let Some(t) = st.tile(id).cloned() else { return };
            match st.face().free_spot(st.page, t.w, t.h) {
                Some((x, y)) => {
                    let copy = DeckTile { id: st.new_id(), x, y, ..t };
                    st.picked = Some(copy.id.clone());
                    let page = st.page;
                    st.face_mut().pages[page].push(copy);
                    true
                }
                None => false,
            }
        };
        if done {
            self.edited(true);
        } else {
            self.flash(t!("This page is full. Add a page with the + under the grid."));
        }
    }

    fn set_label(self: &Rc<Self>, id: &str, text: &str) {
        if let Some(t) = self.st.borrow_mut().tile_mut(id) {
            t.label = text.chars().take(MAX_LABEL).collect();
        }
        self.render_grid(false);
        self.render_state();
    }

    fn set_color(self: &Rc<Self>, id: &str, color: &str) {
        if let Some(t) = self.st.borrow_mut().tile_mut(id) {
            t.color = color.into();
        }
        self.render_grid(false);
        self.render_state();
    }

    /// Runs another action; a label that was the old action's name follows it.
    fn set_action(self: &Rc<Self>, id: &str, action: &str, name: &str) {
        {
            let mut st = self.st.borrow_mut();
            let old = st.tile(id).and_then(|t| st.prefs.actions.iter().find(|o| Some(&o.action.id) == t.action.as_ref())).map(|o| o.action.name.clone());
            let Some(t) = st.tile_mut(id) else { return };
            if t.action.as_deref() == Some(action) {
                return;
            }
            if old.as_deref() == Some(t.label.as_str()) || t.label.is_empty() {
                t.label = name.chars().take(MAX_LABEL).collect();
            }
            t.action = Some(action.into());
            t.color = model::default_color(&t.kind, Some(action)).into();
        }
        self.edited(false);
    }

    fn set_theme(self: &Rc<Self>, theme: Theme) {
        {
            let mut st = self.st.borrow_mut();
            let layout = st.layout;
            st.layouts.layouts[layout].theme.id = theme.id().into();
        }
        self.edited(false);
    }

    fn set_accent(self: &Rc<Self>, accent: &str) {
        {
            let mut st = self.st.borrow_mut();
            let layout = st.layout;
            st.layouts.layouts[layout].theme.accent = accent.into();
        }
        style::load(accent);
        let st = self.st.borrow();
        let now = now();
        let build = |t: &DeckTile| self.build_tile(&st, t, &now);
        self.ui.grid.rebuild(|_| true, &build);
        drop(st);
        self.render_state();
    }

    fn rename(self: &Rc<Self>, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            let layout = st.layout;
            st.layouts.layouts[layout].name = name.chars().take(MAX_LABEL).collect();
        }
        self.render_chrome();
    }

    fn select_layout(self: &Rc<Self>, i: usize) {
        {
            let mut st = self.st.borrow_mut();
            if i >= st.layouts.layouts.len() || i == st.layout {
                return;
            }
            (st.layout, st.page, st.picked) = (i, 0, None);
        }
        self.edited(false);
    }

    fn new_layout(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.layouts.layouts.len() >= MAX_LAYOUTS {
                return;
            }
            let (portrait, landscape) = (st.current().portrait.grid, st.current().landscape.grid);
            let layout = DeckLayout {
                id: st.new_id(),
                name: t!("Layout {n}", n = st.layouts.layouts.len() + 1),
                theme: DeckTheme::default(),
                portrait: DeckFace { grid: portrait, pages: vec![Vec::new()] },
                landscape: DeckFace { grid: landscape, pages: vec![Vec::new()] },
            };
            st.layouts.layouts.push(layout);
            (st.layout, st.page, st.picked) = (st.layouts.layouts.len() - 1, 0, None);
        }
        self.edited(false);
    }

    fn duplicate_layout(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.layouts.layouts.len() >= MAX_LAYOUTS {
                return;
            }
            let mut copy = st.current().clone();
            let mut taken = model::ids(&st.layouts);
            let mut fresh = || {
                let id = model::new_id(|id| taken.contains(id));
                taken.insert(id.clone());
                id
            };
            copy.id = fresh();
            for face in [&mut copy.portrait, &mut copy.landscape] {
                for t in face.pages.iter_mut().flatten() {
                    t.id = fresh();
                }
            }
            let name = t!("{name} Copy", name = copy.name.as_str());
            copy.name = name.chars().take(MAX_LABEL).collect();
            st.layouts.layouts.push(copy);
            (st.layout, st.page, st.picked) = (st.layouts.layouts.len() - 1, 0, None);
        }
        self.edited(false);
    }

    fn delete_layout(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.layouts.layouts.len() < 2 {
                return;
            }
            let i = st.layout;
            let gone = st.layouts.layouts.remove(i);
            if st.layouts.active == gone.id {
                st.layouts.active = st.layouts.layouts[0].id.clone();
            }
            (st.layout, st.page, st.picked) = (i.saturating_sub(1), 0, None);
        }
        self.edited(false);
    }

    fn set_orientation(self: &Rc<Self>, landscape: bool) {
        {
            let mut st = self.st.borrow_mut();
            if st.landscape == landscape {
                return;
            }
            (st.landscape, st.page, st.picked) = (landscape, 0, None);
        }
        self.edited(false);
    }

    fn go_page(self: &Rc<Self>, page: usize) {
        {
            let mut st = self.st.borrow_mut();
            (st.page, st.picked) = (page, None);
        }
        self.edited(false);
    }

    fn add_page(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.face().pages.len() >= MAX_PAGES {
                return;
            }
            st.face_mut().pages.push(Vec::new());
            (st.page, st.picked) = (st.face().pages.len() - 1, None);
        }
        self.edited(false);
    }

    fn remove_page(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            let page = st.page;
            if st.face().pages.len() < 2 || !st.tiles().is_empty() {
                return;
            }
            st.face_mut().pages.remove(page);
            st.page = page.saturating_sub(1);
        }
        self.edited(false);
    }

    /// The landscape face from the portrait one, in its order.
    fn reflow(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            let mut taken = model::ids(&st.layouts);
            let layout = st.layout;
            let l = &mut st.layouts.layouts[layout];
            l.landscape = model::reflow(&l.portrait, l.landscape.grid, || {
                let id = model::new_id(|id| taken.contains(id));
                taken.insert(id.clone());
                id
            });
            (st.page, st.picked) = (0, None);
        }
        self.edited(false);
    }

    // ---- the service ----

    fn allow(self: &Rc<Self>, action: &str, row: &adw::SwitchRow, reverting: Rc<Cell<bool>>) {
        let (Some(win), on) = (self.win.upgrade(), row.is_active()) else { return };
        let Some(c) = win.client() else { return };
        let (phone, action) = (self.phone, action.to_string());
        let a = action.clone();
        let call = win.bridge.call(&c, move |c| async move { c.set_deck_action(phone.to_string(), a, on).await });
        let (e, row) = (Rc::downgrade(self), row.clone());
        glib::spawn_future_local(async move {
            match call.await {
                Ok(()) => {
                    if let Some(e) = e.upgrade()
                        && let Some(o) = e.st.borrow_mut().prefs.actions.iter_mut().find(|o| o.action.id == action)
                    {
                        o.on = on;
                    }
                }
                Err(err) => {
                    win.toast(&fmt::capitalize(&err));
                    reverting.set(true);
                    row.set_active(!on);
                    reverting.set(false);
                }
            }
        });
    }

    fn add_command(self: &Rc<Self>) {
        if let Some(win) = self.win.upgrade() {
            let commands = self.st.borrow().prefs.commands.clone();
            win.ask_deck_command(self.phone, commands);
        }
    }

    /// The command goes, and its tiles from every layout with it.
    fn remove_command(self: &Rc<Self>, id: &str) {
        let Some(win) = self.win.upgrade() else { return };
        let action = format!("command.{id}");
        let mut removed = 0;
        let rest = {
            let mut st = self.st.borrow_mut();
            for l in st.layouts.layouts.iter_mut() {
                for face in [&mut l.portrait, &mut l.landscape] {
                    for page in face.pages.iter_mut() {
                        let before = page.len();
                        page.retain(|t| t.action.as_deref() != Some(action.as_str()));
                        removed += before - page.len();
                    }
                }
            }
            st.prefs.commands.iter().filter(|c| c.id != id).cloned().collect()
        };
        self.edited(true);
        match removed {
            0 => {}
            1 => win.toast(t!("Its tile is gone from the deck too. Save to keep it that way.")),
            n => win.toast(&t!("Its {n} tiles are gone from the deck too. Save to keep it that way.", n)),
        }
        win.save_deck_commands(self.phone, rest);
    }

    /// Sends every layout, after the checks the service makes; `then` once saved.
    pub fn save(self: &Rc<Self>, then: Option<Box<dyn FnOnce()>>) {
        let Some(win) = self.win.upgrade() else { return };
        let Some(c) = win.client() else { return };
        let checked = {
            let mut st = self.st.borrow_mut();
            let id = st.current().id.clone();
            st.layouts.active = id;
            let st = &*st;
            let before = st.saved_runs();
            let ok = |a: &str| st.offered(a) || before.contains(a);
            let missing: Vec<String> = st.layouts.layouts.iter().flat_map(|l| [&l.portrait, &l.landscape]).flat_map(|f| f.pages.iter().flatten()).filter(|t| t.action.as_deref().is_some_and(|a| !ok(a))).map(|t| format!("\u{201c}{}\u{201d}", st.name_of(t))).collect();
            if !missing.is_empty() {
                Err(t!("This computer doesn't offer what {tiles} run. Delete them, then save.", tiles = missing.join(", ")))
            } else {
                st.layouts.check(ok).map(|()| st.layouts.clone()).map_err(|why| t!("This deck can't be saved: {why}", why = why.as_str()))
            }
        };
        let sent = match checked {
            Ok(sent) => sent,
            Err(why) => {
                win.toast(&why);
                self.render_state();
                return;
            }
        };
        self.st.borrow_mut().saving = true;
        self.render_state();
        let (phone, body) = (self.phone, sent.clone());
        let call = win.bridge.call(&c, move |c| async move {
            let saved = c.set_deck_layouts(phone.to_string(), body).await?;
            let prefs = c.deck_prefs(phone.to_string()).await?;
            Ok((saved, prefs))
        });
        let e = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(e) = e.upgrade() else { return };
            e.st.borrow_mut().saving = false;
            match call.await {
                Ok((saved, prefs)) => {
                    {
                        let mut st = e.st.borrow_mut();
                        if st.layouts == sent && !saved.layouts.is_empty() {
                            st.layouts = saved.clone();
                        } else {
                            st.layouts.version = saved.version;
                        }
                        (st.saved, st.kept) = (saved, true);
                        st.settle();
                    }
                    e.set_prefs(prefs);
                    e.render_chrome();
                    if let Some(then) = then {
                        then();
                    }
                }
                Err(err) => {
                    win.toast(&fmt::capitalize(&err));
                    e.render_state();
                }
            }
        });
    }
}

/// A library item's key, the same for the same widget whatever the language.
fn key(item: &Item) -> String {
    format!("{}|{}", item.kind, item.action.as_deref().unwrap_or(""))
}

fn build_ui() -> (adw::BreakpointBin, Ui) {
    // The bar along the top
    let bar = gtk::Box::builder().spacing(6).css_classes(["deck-bar"]).build();
    let library_toggle = gtk::ToggleButton::builder().icon_name("view-grid-symbolic").tooltip_text(t!("Widgets")).active(true).visible(false).build();
    let names = gtk::StringList::new(&[]);
    let layouts = gtk::DropDown::builder().model(&names).tooltip_text(t!("Layout")).build();
    let new_layout = gtk::Button::with_label(t!("New Layout"));
    let new_icon = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text(t!("New Layout")).visible(false).build();
    let portrait = gtk::ToggleButton::with_label(t!("Portrait"));
    let landscape = gtk::ToggleButton::builder().label(t!("Landscape")).group(&portrait).build();
    let orientation = gtk::Box::builder().css_classes(["linked"]).build();
    orientation.append(&portrait);
    orientation.append(&landscape);
    let chip = gtk::Label::builder().css_classes(["deck-chip"]).build();
    let save = gtk::Button::builder().label(t!("Save")).css_classes(["suggested-action"]).build();
    bar.append(&library_toggle);
    bar.append(&layouts);
    bar.append(&new_layout);
    bar.append(&new_icon);
    bar.append(&orientation);
    bar.append(&gtk::Box::builder().hexpand(true).build());
    bar.append(&chip);
    bar.append(&save);

    // The library
    let search = gtk::SearchEntry::builder().placeholder_text(t!("Search widgets")).build();
    let groups = column(2);
    let library = column(10);
    library.add_css_class("deck-library");
    library.append(&search);
    library.append(&label(t!("Drag a widget onto the phone, or click it to add it where there's room."), &["deck-hint"]));
    library.append(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&groups).build());

    // The stage: hints, themes, the phone
    let info = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).css_classes(["deck-note"]).build();
    let note = gtk::Label::builder().xalign(1.0).justify(gtk::Justification::Right).wrap(true).css_classes(["deck-note"]).build();
    let notes = gtk::Box::builder().spacing(10).margin_top(12).margin_start(14).margin_end(14).build();
    notes.append(&info);
    notes.append(&note);
    let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).halign(gtk::Align::Center).column_spacing(6).row_spacing(6).max_children_per_line(5).margin_top(10).margin_start(14).margin_end(14).css_classes(["deck-themes"]).build();
    let mut themes: Vec<(Theme, gtk::ToggleButton)> = Vec::new();
    for theme in Theme::ALL {
        let inside = gtk::Box::builder().spacing(7).build();
        inside.append(&gtk::Box::builder().css_classes(["sw", &format!("sw-{}", theme.id())]).valign(gtk::Align::Center).build());
        inside.append(&gtk::Label::new(Some(theme.name())));
        let button = gtk::ToggleButton::builder().child(&inside).css_classes(["deck-theme"]).build();
        if let Some((_, first)) = themes.first() {
            button.set_group(Some(first));
        }
        flow.append(&button);
        themes.push((theme, button));
    }
    let time = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    let status = gtk::Box::builder().css_classes(["deck-status"]).build();
    status.append(&time);
    let battery = gtk::Label::new(None);
    status.append(&battery);
    let deck_name = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(pango::EllipsizeMode::End).css_classes(["layout"]).build();
    let page_info = gtk::Label::builder().css_classes(["pageinfo"]).build();
    let name_row = gtk::Box::builder().css_classes(["deck-name"]).build();
    name_row.append(&deck_name);
    name_row.append(&page_info);
    let grid = Grid::default();
    grid.set_vexpand(true);
    let dots = gtk::Box::builder().halign(gtk::Align::Center).css_classes(["deck-dots"]).build();
    let screen = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).overflow(gtk::Overflow::Hidden).css_classes(["deck-screen"]).build();
    screen.append(&status);
    screen.append(&name_row);
    screen.append(&grid);
    screen.append(&dots);
    let phone = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["deck-phone"]).build();
    phone.append(&screen);
    let fit = Fit::new(&phone);
    fit.set_size(PORTRAIT.0, PORTRAIT.1);
    fit.set_hexpand(true);
    fit.set_vexpand(true);
    fit.set_margin_top(16);
    fit.set_margin_bottom(24);
    fit.set_margin_start(14);
    fit.set_margin_end(14);
    let stage = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).vexpand(true).css_classes(["deck-stage"]).build();
    stage.append(&notes);
    stage.append(&flow);
    stage.append(&fit);

    // The inspector
    let inspector = column(12);
    inspector.add_css_class("deck-inspector-box");
    let inspector_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).width_request(270).propagate_natural_height(true).child(&inspector).css_classes(["deck-inspector"]).build();

    let work = gtk::Box::builder().css_classes(["deck-work"]).build();
    work.append(&stage);
    work.append(&inspector_scroll);
    let work_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vscrollbar_policy(gtk::PolicyType::Never).child(&work).build();
    let split = adw::OverlaySplitView::builder().sidebar(&library).content(&work_scroll).min_sidebar_width(250.0).max_sidebar_width(270.0).build();
    library_toggle.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();

    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    body.append(&bar);
    split.set_vexpand(true);
    body.append(&split);
    let root = adw::BreakpointBin::builder().width_request(360).height_request(420).child(&body).build();

    // Narrow: the library over the page on demand, the inspector under the phone
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 860sp").expect("breakpoint"));
    narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
    narrow.add_setter(&split, "show-sidebar", Some(&false.to_value()));
    narrow.add_setter(&library_toggle, "visible", Some(&true.to_value()));
    narrow.add_setter(&new_layout, "visible", Some(&false.to_value()));
    narrow.add_setter(&new_icon, "visible", Some(&true.to_value()));
    narrow.add_setter(&chip, "visible", Some(&false.to_value()));
    narrow.add_setter(&work, "orientation", Some(&gtk::Orientation::Vertical.to_value()));
    narrow.add_setter(&work, "css-classes", Some(&["deck-work", "narrow"].to_value()));
    narrow.add_setter(&work_scroll, "vscrollbar-policy", Some(&gtk::PolicyType::Automatic.to_value()));
    narrow.add_setter(&inspector_scroll, "vscrollbar-policy", Some(&gtk::PolicyType::Never.to_value()));
    narrow.add_setter(&inspector_scroll, "width-request", Some(&(-1).to_value()));
    narrow.add_setter(&stage, "height-request", Some(&640.to_value()));
    root.add_breakpoint(narrow);

    let ui = Ui {
        names,
        layouts,
        portrait,
        landscape,
        chip,
        save,
        split,
        search,
        groups,
        themes,
        info,
        note,
        fit,
        phone,
        screen,
        status,
        time,
        deck_name,
        page_info,
        grid,
        dots,
        inspector,
        new_layout,
        new_icon,
        battery,
    };
    (root, ui)
}
